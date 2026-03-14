use std::path::Path;
use tokio::fs;

use crate::client::{BorgClient, UPLOAD_CHUNK_SIZE, DEFAULT_POLL_INTERVAL_MS};
use crate::types::*;
use crate::BorgError;

pub fn guess_mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("md") => "text/markdown",
        Some("json") => "application/json",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("xml") => "application/xml",
        Some("pdf") => "application/pdf",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

pub fn is_expected_text_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "json" | "txt" | "csv" | "xml" | "pdf" | "doc" | "docx" | "xlsx" | "pptx")
    )
}

async fn list_files_recursive(dir: &Path) -> Result<Vec<std::path::PathBuf>, BorgError> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let mut entries = fs::read_dir(&current)
            .await
            .map_err(|e| BorgError::Request(format!("read_dir {}: {e}", current.display())))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| BorgError::Request(e.to_string()))?
        {
            let path = entry.path();
            let ft = entry
                .file_type()
                .await
                .map_err(|e| BorgError::Request(e.to_string()))?;
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

pub async fn upload_directory(
    client: &BorgClient,
    project_id: i64,
    dir: &Path,
    chunk_size: Option<usize>,
    privileged: bool,
) -> Result<Vec<UploadedFile>, BorgError> {
    let chunk_size = chunk_size.unwrap_or(UPLOAD_CHUNK_SIZE);
    let mut uploaded = Vec::new();

    for file_path in list_files_recursive(dir).await? {
        let bytes = fs::read(&file_path)
            .await
            .map_err(|e| BorgError::Request(format!("read {}: {e}", file_path.display())))?;

        let relative = file_path
            .strip_prefix(dir)
            .unwrap_or(&file_path)
            .to_string_lossy()
            .replace('\\', "/");

        let mime_type = guess_mime_type(&file_path).to_string();
        let total_chunks = std::cmp::max(1, (bytes.len() + chunk_size - 1) / chunk_size) as i64;

        let session = client
            .create_upload_session(
                project_id,
                &CreateUploadSessionBody {
                    file_name: relative.clone(),
                    mime_type: Some(mime_type.clone()),
                    file_size: bytes.len() as i64,
                    chunk_size: chunk_size as i64,
                    total_chunks,
                    is_zip: Some(false),
                    privileged: Some(privileged),
                },
            )
            .await?;

        for i in 0..total_chunks {
            let start = (i as usize) * chunk_size;
            let end = std::cmp::min(start + chunk_size, bytes.len());
            client
                .upload_chunk(project_id, session.session_id, i, &bytes[start..end])
                .await?;
        }
        client.complete_upload(project_id, session.session_id).await?;

        uploaded.push(UploadedFile {
            relative_path: relative,
            mime_type,
            size_bytes: bytes.len() as u64,
            expected_text: is_expected_text_file(&file_path),
        });
    }

    Ok(uploaded)
}

pub async fn wait_for_ingestion(
    client: &BorgClient,
    project_id: i64,
    uploaded_files: &[UploadedFile],
    timeout_ms: u64,
    poll_interval_ms: Option<u64>,
) -> Result<ProjectFilesSummary, BorgError> {
    let poll_interval = poll_interval_ms.unwrap_or(DEFAULT_POLL_INTERVAL_MS);
    let expected_total = uploaded_files.len() as i64;
    let expected_text = uploaded_files.iter().filter(|f| f.expected_text).count() as i64;

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

    while std::time::Instant::now() < deadline {
        let listing = client.list_project_files(project_id, 5).await?;
        if let Some(summary) = listing.summary {
            if summary.total_files >= expected_total && summary.text_files >= expected_text {
                return Ok(summary);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(poll_interval)).await;
    }

    Err(BorgError::Timeout(format!(
        "Timed out waiting for project {project_id} ingestion after {timeout_ms}ms"
    )))
}
