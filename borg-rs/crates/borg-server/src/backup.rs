use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_s3::{primitives::ByteStream, Client};
use borg_core::{
    config::Config,
    db::{Db, ProjectFileRow, ProjectRow, TaskMessage, TaskOutput},
    types::{QueueEntry, Task},
};
use flate2::{write::GzEncoder, Compression};
use tar::Builder;

use crate::storage::FileStorage;

#[derive(Clone)]
enum BackupTarget {
    Disabled,
    S3 {
        bucket: String,
        prefix: String,
        client: Client,
    },
}

#[derive(Debug, serde::Serialize)]
struct ActiveBackupManifest {
    generated_at: String,
    mode: String,
    include_uploads: bool,
    notes: Vec<String>,
    tasks: Vec<TaskBackupRecord>,
    projects: Vec<ProjectBackupRecord>,
}

#[derive(Debug, serde::Serialize)]
struct TaskBackupRecord {
    task: Task,
    outputs: Vec<TaskOutput>,
    messages: Vec<TaskMessage>,
    queue_entries: Vec<QueueEntry>,
    session_archive_key: Option<String>,
    worktree_bundle_key: Option<String>,
    worktree_patch_key: Option<String>,
    untracked_archive_key: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct ProjectBackupRecord {
    project: ProjectRow,
    include_uploads: bool,
    uploads: Vec<ProjectUploadBackupRecord>,
}

#[derive(Debug, serde::Serialize)]
struct ProjectUploadBackupRecord {
    id: i64,
    file_name: String,
    source_path: String,
    stored_path: String,
    size_bytes: i64,
    mime_type: String,
    privileged: bool,
    backup_key: Option<String>,
}

impl BackupTarget {
    async fn from_config(config: &Config) -> Result<Self> {
        if !config.backup_backend.trim().eq_ignore_ascii_case("s3") {
            return Ok(Self::Disabled);
        }
        if config.backup_bucket.trim().is_empty() {
            return Err(anyhow!(
                "backup backend selected but BACKUP_BUCKET/backup_bucket is empty"
            ));
        }
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(config.backup_region.clone()));
        if !config.backup_access_key.is_empty() && !config.backup_secret_key.is_empty() {
            loader = loader.credentials_provider(Credentials::new(
                config.backup_access_key.clone(),
                config.backup_secret_key.clone(),
                None,
                None,
                "borg-backup-config",
            ));
        }
        let shared = loader.load().await;
        let mut s3_builder = aws_sdk_s3::config::Builder::from(&shared);
        if !config.backup_endpoint.trim().is_empty() {
            s3_builder = s3_builder
                .endpoint_url(config.backup_endpoint.clone())
                .force_path_style(true);
        }
        let client = Client::from_conf(s3_builder.build());
        let mut prefix = config.backup_prefix.trim().to_string();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        Ok(Self::S3 {
            bucket: config.backup_bucket.clone(),
            prefix,
            client,
        })
    }

    fn backend_name(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::S3 { .. } => "s3",
        }
    }

    fn target(&self) -> String {
        match self {
            Self::Disabled => "disabled".to_string(),
            Self::S3 { bucket, prefix, .. } => format!("s3://{bucket}/{prefix}"),
        }
    }

    async fn healthcheck(&self) -> Result<()> {
        match self {
            Self::Disabled => Ok(()),
            Self::S3 { bucket, client, .. } => {
                client
                    .head_bucket()
                    .bucket(bucket)
                    .send()
                    .await
                    .context("backup target head_bucket failed")?;
                Ok(())
            },
        }
    }

    async fn put_bytes(&self, key: &str, bytes: Vec<u8>) -> Result<String> {
        match self {
            Self::Disabled => Err(anyhow!("backup target disabled")),
            Self::S3 {
                bucket,
                prefix,
                client,
            } => {
                let object_key = format!("{prefix}{key}");
                client
                    .put_object()
                    .bucket(bucket)
                    .key(&object_key)
                    .body(ByteStream::from(bytes))
                    .send()
                    .await
                    .with_context(|| format!("put backup object {object_key}"))?;
                Ok(format!("s3://{bucket}/{object_key}"))
            },
        }
    }

    async fn put_file(&self, key: &str, path: &Path) -> Result<String> {
        match self {
            Self::Disabled => Err(anyhow!("backup target disabled")),
            Self::S3 {
                bucket,
                prefix,
                client,
            } => {
                let object_key = format!("{prefix}{key}");
                let body = ByteStream::from_path(path)
                    .await
                    .with_context(|| format!("open backup source {}", path.display()))?;
                client
                    .put_object()
                    .bucket(bucket)
                    .key(&object_key)
                    .body(body)
                    .send()
                    .await
                    .with_context(|| format!("put backup object {object_key}"))?;
                Ok(format!("s3://{bucket}/{object_key}"))
            },
        }
    }
}

pub async fn run_backup_loop(db: Arc<Db>, config: Arc<Config>, storage: Arc<FileStorage>) {
    let target = match BackupTarget::from_config(&config).await {
        Ok(target) => target,
        Err(e) => {
            tracing::error!("backup target init failed: {e}");
            let _ = db.set_config("backup_last_error", &e.to_string());
            return;
        },
    };
    let _ = db.set_config("backup_backend_runtime", target.backend_name());
    let _ = db.set_config("backup_target_runtime", &target.target());

    if matches!(target, BackupTarget::Disabled) {
        tracing::info!("backup backend: disabled");
        return;
    }

    if let Err(e) = target.healthcheck().await {
        tracing::error!("backup target healthcheck failed: {e}");
        let _ = db.set_config("backup_last_error", &e.to_string());
    }

    let interval_s = config.backup_poll_interval_s.max(30) as u64;
    loop {
        let started_at = chrono::Utc::now();
        let _ = db.set_config("backup_last_started_at", &started_at.to_rfc3339());
        match backup_once(&db, &config, &storage, &target).await {
            Ok(summary) => {
                let _ = db.set_config("backup_last_error", "");
                let _ = db.set_config("backup_last_success_at", &chrono::Utc::now().to_rfc3339());
                let _ = db.set_config("backup_last_summary", &summary);
            },
            Err(e) => {
                tracing::error!("backup loop failed: {e}");
                let _ = db.set_config("backup_last_error", &e.to_string());
            },
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval_s)).await;
    }
}

pub async fn backup_status_snapshot(db: &Db, config: &Config) -> serde_json::Value {
    let last_started = db
        .get_config("backup_last_started_at")
        .ok()
        .flatten()
        .unwrap_or_default();
    let last_success = db
        .get_config("backup_last_success_at")
        .ok()
        .flatten()
        .unwrap_or_default();
    let last_error = db
        .get_config("backup_last_error")
        .ok()
        .flatten()
        .unwrap_or_default();
    let last_summary = db
        .get_config("backup_last_summary")
        .ok()
        .flatten()
        .unwrap_or_default();
    let runtime_backend = db
        .get_config("backup_backend_runtime")
        .ok()
        .flatten()
        .unwrap_or_else(|| config.backup_backend.clone());
    let runtime_target = db
        .get_config("backup_target_runtime")
        .ok()
        .flatten()
        .unwrap_or_default();
    serde_json::json!({
        "configured_backend": config.backup_backend,
        "runtime_backend": runtime_backend,
        "target": runtime_target,
        "mode": config.backup_mode,
        "interval_s": config.backup_poll_interval_s,
        "enabled": config.backup_backend.eq_ignore_ascii_case("s3"),
        "last_started_at": last_started,
        "last_success_at": last_success,
        "last_error": last_error,
        "last_summary": last_summary,
    })
}

async fn backup_once(
    db: &Db,
    config: &Config,
    storage: &FileStorage,
    target: &BackupTarget,
) -> Result<String> {
    let include_uploads = config.backup_mode.eq_ignore_ascii_case("include_uploads");
    let tasks = db.list_active_tasks().context("list active tasks for backup")?;
    let mut task_records = Vec::with_capacity(tasks.len());
    let mut project_records = Vec::new();
    let mut seen_projects = std::collections::HashSet::new();

    for task in tasks {
        let outputs = db.get_task_outputs(task.id).unwrap_or_default();
        let messages = db.get_task_messages(task.id).unwrap_or_default();
        let queue_entries = db.get_queue_entries_for_task(task.id).unwrap_or_default();

        let session_archive_key = maybe_archive_session_dir(config, target, task.id).await?;
        let worktree_bundle_key = maybe_backup_worktree_bundle(target, &task).await?;
        let worktree_patch_key = maybe_backup_worktree_patch(target, &task).await?;
        let untracked_archive_key = maybe_backup_untracked_files(target, &task).await?;

        if task.project_id > 0 && seen_projects.insert(task.project_id) {
            if let Some(project) = db.get_project(task.project_id).context("get project for backup")?
            {
                let uploads = backup_project_uploads(
                    db,
                    storage,
                    target,
                    &project,
                    include_uploads,
                )
                .await?;
                project_records.push(ProjectBackupRecord {
                    project,
                    include_uploads,
                    uploads,
                });
            }
        }

        task_records.push(TaskBackupRecord {
            task,
            outputs,
            messages,
            queue_entries,
            session_archive_key,
            worktree_bundle_key,
            worktree_patch_key,
            untracked_archive_key,
        });
    }

    let notes = if include_uploads {
        vec![
            "Uploaded source files are included because backup_mode=include_uploads.".to_string(),
        ]
    } else {
        vec![
            "Uploaded source files are excluded by default to control storage cost.".to_string(),
            "Restoring active work may still require users to re-upload source materials."
                .to_string(),
        ]
    };

    let manifest = ActiveBackupManifest {
        generated_at: chrono::Utc::now().to_rfc3339(),
        mode: config.backup_mode.clone(),
        include_uploads,
        notes,
        tasks: task_records,
        projects: project_records,
    };
    let payload = serde_json::to_vec_pretty(&manifest).context("serialize active backup manifest")?;
    let key = "active-work/manifest.json";
    target.put_bytes(key, payload).await?;

    Ok(format!(
        "protected {} active task(s), {} active project(s), uploads_included={}",
        manifest.tasks.len(),
        manifest.projects.len(),
        include_uploads
    ))
}

async fn maybe_archive_session_dir(
    config: &Config,
    target: &BackupTarget,
    task_id: i64,
) -> Result<Option<String>> {
    let session_dir = format!("{}/sessions/task-{}", config.data_dir, task_id);
    let session_path = PathBuf::from(session_dir);
    if !session_path.exists() {
        return Ok(None);
    }
    let archive_path = tokio::task::spawn_blocking(move || archive_directory(&session_path, "session"))
        .await
        .context("join session archive task")??;
    let Some(archive_path) = archive_path else {
        return Ok(None);
    };
    let key = format!("active-work/tasks/{task_id}/session.tar.gz");
    let uploaded = target.put_file(&key, &archive_path).await?;
    let _ = tokio::fs::remove_file(&archive_path).await;
    Ok(Some(uploaded))
}

async fn maybe_backup_worktree_bundle(target: &BackupTarget, task: &Task) -> Result<Option<String>> {
    let task = task.clone();
    let task_for_bundle = task.clone();
    let bundle_path = tokio::task::spawn_blocking(move || create_worktree_bundle(&task_for_bundle))
        .await
        .context("join worktree bundle task")??;
    let Some(bundle_path) = bundle_path else {
        return Ok(None);
    };
    let key = format!("active-work/tasks/{}/worktree.bundle", task.id);
    let uploaded = target.put_file(&key, &bundle_path).await?;
    let _ = tokio::fs::remove_file(&bundle_path).await;
    Ok(Some(uploaded))
}

async fn maybe_backup_worktree_patch(target: &BackupTarget, task: &Task) -> Result<Option<String>> {
    let task = task.clone();
    let task_for_patch = task.clone();
    let patch_path = tokio::task::spawn_blocking(move || create_worktree_patch(&task_for_patch))
        .await
        .context("join worktree patch task")??;
    let Some(patch_path) = patch_path else {
        return Ok(None);
    };
    let key = format!("active-work/tasks/{}/worktree.patch", task.id);
    let uploaded = target.put_file(&key, &patch_path).await?;
    let _ = tokio::fs::remove_file(&patch_path).await;
    Ok(Some(uploaded))
}

async fn maybe_backup_untracked_files(target: &BackupTarget, task: &Task) -> Result<Option<String>> {
    let task = task.clone();
    let task_for_archive = task.clone();
    let archive_path = tokio::task::spawn_blocking(move || create_untracked_archive(&task_for_archive))
        .await
        .context("join untracked archive task")??;
    let Some(archive_path) = archive_path else {
        return Ok(None);
    };
    let key = format!("active-work/tasks/{}/untracked.tar.gz", task.id);
    let uploaded = target.put_file(&key, &archive_path).await?;
    let _ = tokio::fs::remove_file(&archive_path).await;
    Ok(Some(uploaded))
}

async fn backup_project_uploads(
    db: &Db,
    storage: &FileStorage,
    target: &BackupTarget,
    project: &ProjectRow,
    include_uploads: bool,
) -> Result<Vec<ProjectUploadBackupRecord>> {
    let files = db.list_project_files(project.id).unwrap_or_default();
    let mut uploads = Vec::with_capacity(files.len());
    for file in files {
        let backup_key = if include_uploads {
            backup_project_file(storage, target, project.id, &file).await?
        } else {
            None
        };
        uploads.push(ProjectUploadBackupRecord {
            id: file.id,
            file_name: file.file_name.clone(),
            source_path: file.source_path.clone(),
            stored_path: file.stored_path.clone(),
            size_bytes: file.size_bytes,
            mime_type: file.mime_type.clone(),
            privileged: file.privileged,
            backup_key,
        });
    }
    Ok(uploads)
}

async fn backup_project_file(
    storage: &FileStorage,
    target: &BackupTarget,
    project_id: i64,
    file: &ProjectFileRow,
) -> Result<Option<String>> {
    let bytes = match storage.read_all(&file.stored_path).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!("backup read for project file {} failed: {e}", file.id);
            return Ok(None);
        },
    };
    let object_name = format!(
        "active-work/projects/{project_id}/uploads/{}-{}",
        file.id,
        safe_name(&file.file_name)
    );
    let uploaded = target.put_bytes(&object_name, bytes).await?;
    Ok(Some(uploaded))
}

fn archive_directory(dir: &Path, label: &str) -> Result<Option<PathBuf>> {
    if !dir.exists() {
        return Ok(None);
    }
    let out_path = temp_output_path(label, "tar.gz");
    let out = File::create(&out_path)
        .with_context(|| format!("create archive {}", out_path.display()))?;
    let encoder = GzEncoder::new(out, Compression::default());
    let mut tar = Builder::new(encoder);
    tar.append_dir_all(".", dir)
        .with_context(|| format!("archive directory {}", dir.display()))?;
    let encoder = tar.into_inner().context("finalize tar stream")?;
    encoder.finish().context("finish gzip stream")?;
    Ok(Some(out_path))
}

fn create_worktree_bundle(task: &Task) -> Result<Option<PathBuf>> {
    if task.branch.trim().is_empty() || !Path::new(&task.repo_path).exists() {
        return Ok(None);
    }
    let base_ref = ["origin/main", "main", "master"]
        .into_iter()
        .find(|candidate| git_ok(&task.repo_path, &["rev-parse", "--verify", candidate]));
    let Some(base_ref) = base_ref else {
        return Ok(None);
    };
    let out_path = temp_output_path("worktree", "bundle");
    let spec = format!("{base_ref}..{}", task.branch);
    let output = std::process::Command::new("git")
        .args(["-C", &task.repo_path, "bundle", "create"])
        .arg(&out_path)
        .arg(&spec)
        .output()
        .context("run git bundle create")?;
    if !output.status.success() || std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(&out_path);
        return Ok(None);
    }
    Ok(Some(out_path))
}

fn create_worktree_patch(task: &Task) -> Result<Option<PathBuf>> {
    if !Path::new(&task.repo_path).exists() {
        return Ok(None);
    }
    let output = std::process::Command::new("git")
        .args(["-C", &task.repo_path, "diff", "--binary", "HEAD"])
        .output()
        .context("run git diff --binary HEAD")?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);
    }
    let out_path = temp_output_path("worktree", "patch");
    std::fs::write(&out_path, output.stdout)
        .with_context(|| format!("write patch {}", out_path.display()))?;
    Ok(Some(out_path))
}

fn create_untracked_archive(task: &Task) -> Result<Option<PathBuf>> {
    if !Path::new(&task.repo_path).exists() {
        return Ok(None);
    }
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &task.repo_path,
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .context("run git ls-files --others")?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);
    }
    let files: Vec<PathBuf> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| PathBuf::from(String::from_utf8_lossy(entry).to_string()))
        .collect();
    if files.is_empty() {
        return Ok(None);
    }
    let repo_path = PathBuf::from(&task.repo_path);
    let out_path = temp_output_path("untracked", "tar.gz");
    let out = File::create(&out_path)
        .with_context(|| format!("create untracked archive {}", out_path.display()))?;
    let encoder = GzEncoder::new(out, Compression::default());
    let mut tar = Builder::new(encoder);
    for relative in files {
        let absolute = repo_path.join(&relative);
        if absolute.is_file() {
            tar.append_path_with_name(&absolute, &relative)
                .with_context(|| format!("append untracked file {}", absolute.display()))?;
        }
    }
    let encoder = tar.into_inner().context("finalize untracked tar stream")?;
    encoder.finish().context("finish untracked gzip stream")?;
    Ok(Some(out_path))
}

fn git_ok(repo_path: &str, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(["-C", repo_path])
        .args(args)
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn temp_output_path(label: &str, ext: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "borg-{}-{}-{}.{}",
        label,
        std::process::id(),
        rand::random::<u64>(),
        ext
    ));
    path
}

fn safe_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('.').trim_matches('_');
    if trimmed.is_empty() {
        "upload.bin".to_string()
    } else {
        trimmed.to_string()
    }
}
