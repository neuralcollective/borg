/// Integration test for check_remote_updates multi-repo loop correctness.
///
/// Verifies that when the first `is_self` repo is already up-to-date, the
/// loop continues to the second repo instead of returning early (the bug).
use std::{collections::HashMap, sync::Arc};

use borg_core::{config::Config, db::Db, pipeline::Pipeline, sandbox::SandboxMode, types::RepoConfig};

fn git_in(dir: &str, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    if !out.status.success() {
        panic!(
            "git -C {} {} failed:\n{}",
            dir,
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn git_run(args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .output()
        .expect("spawn git");
    if !out.status.success() {
        panic!(
            "git {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn make_config(data_dir: &str, repos: Vec<RepoConfig>, build_cmd: String) -> Arc<Config> {
    Arc::new(Config {
        telegram_token: String::new(),
        oauth_token: String::new(),
        assistant_name: String::new(),
        trigger_pattern: String::new(),
        data_dir: data_dir.to_string(),
        container_image: String::new(),
        model: String::new(),
        credentials_path: String::new(),
        session_max_age_hours: 24,
        max_consecutive_errors: 3,
        pipeline_repo: String::new(),
        pipeline_test_cmd: String::new(),
        pipeline_lint_cmd: String::new(),
        backend: String::new(),
        pipeline_admin_chat: String::new(),
        release_interval_mins: 60,
        continuous_mode: false,
        chat_collection_window_ms: 500,
        chat_cooldown_ms: 1000,
        agent_timeout_s: 300,
        max_chat_agents: 5,
        chat_rate_limit: 10,
        pipeline_max_agents: 3,
        web_bind: "127.0.0.1".into(),
        web_port: 8080,
        dashboard_dist_dir: String::new(),
        container_setup: String::new(),
        container_memory_mb: 512,
        container_cpus: 0.0,
        sandbox_backend: "direct".into(),
        pipeline_max_backlog: 100,
        pipeline_seed_cooldown_s: 60,
        proposal_promote_threshold: 3,
        pipeline_tick_s: 5,
        remote_check_interval_s: 0,
        mirror_refresh_interval_s: 60,
        pipeline_agent_cooldown_s: 120,
        git_author_name: "Test".into(),
        git_author_email: "test@test.com".into(),
        git_committer_name: "Test".into(),
        git_committer_email: "test@test.com".into(),
        git_via_borg: false,
        git_claude_coauthor: false,
        git_user_coauthor: String::new(),
        watched_repos: repos,
        build_cmd,
        self_update_enabled: true,
        codex_api_key: String::new(),
        codex_credentials_path: String::new(),
        discord_token: String::new(),
        wa_auth_dir: String::new(),
        wa_disabled: true,
        observer_config: String::new(),
    })
}

fn self_repo(path: &str) -> RepoConfig {
    RepoConfig {
        path: path.to_string(),
        test_cmd: String::new(),
        prompt_file: String::new(),
        mode: "sweborg".into(),
        is_self: true,
        auto_merge: false,
        lint_cmd: String::new(),
        backend: String::new(),
        repo_slug: String::new(),
    }
}

/// Set up `repo_a` as a plain git repo that is in sync with its mirror.
/// Returns the absolute path as a String.
fn setup_uptodate_repo(base: &std::path::Path, mirrors_dir: &std::path::Path) -> String {
    let repo = base.join("repo_a");
    std::fs::create_dir_all(&repo).expect("create repo_a");
    let repo_str = repo.to_str().expect("path");

    git_in(repo_str, &["init"]);
    git_in(repo_str, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git_in(repo_str, &[
        "-c", "user.email=t@t.com",
        "-c", "user.name=T",
        "commit", "--allow-empty", "-m", "init",
    ]);

    // Clone bare as mirror → same HEAD → up-to-date
    let mirror = mirrors_dir.join("repo_a.git");
    git_run(&["clone", "--bare", repo_str, mirror.to_str().expect("path")]);

    repo_str.to_string()
}

/// Set up `repo_b` such that its mirror is one commit ahead (simulating a remote update).
/// Returns the absolute path of the working repo as a String.
fn setup_behind_repo(base: &std::path::Path, mirrors_dir: &std::path::Path) -> String {
    // Create a bare "origin" for repo_b
    let origin = base.join("origin_b.git");
    git_run(&["init", "--bare", origin.to_str().expect("path")]);
    git_in(origin.to_str().expect("path"), &[
        "symbolic-ref", "HEAD", "refs/heads/main",
    ]);

    // Create a working scratch repo, commit, push to origin
    let scratch = base.join("scratch_b");
    std::fs::create_dir_all(&scratch).expect("create scratch_b");
    let scratch_str = scratch.to_str().expect("path");
    git_in(scratch_str, &["init"]);
    git_in(scratch_str, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git_in(scratch_str, &["remote", "add", "origin", origin.to_str().expect("path")]);
    git_in(scratch_str, &[
        "-c", "user.email=t@t.com",
        "-c", "user.name=T",
        "commit", "--allow-empty", "-m", "init",
    ]);
    git_in(scratch_str, &["push", "origin", "main"]);

    // Clone origin to repo_b (the working dir the pipeline watches)
    let repo_b = base.join("repo_b");
    git_run(&["clone", origin.to_str().expect("path"), repo_b.to_str().expect("path")]);

    // Advance origin by one more commit (repo_b is now behind)
    git_in(scratch_str, &[
        "-c", "user.email=t@t.com",
        "-c", "user.name=T",
        "commit", "--allow-empty", "-m", "advance",
    ]);
    git_in(scratch_str, &["push", "origin", "main"]);

    // Create mirror from origin (has the newer commit)
    let mirror = mirrors_dir.join("repo_b.git");
    git_run(&["clone", "--bare", origin.to_str().expect("path"), mirror.to_str().expect("path")]);

    repo_b.to_str().expect("path").to_string()
}

/// When there are two `is_self` repos and the first is up-to-date, the loop
/// must `continue` to the second instead of returning early.
///
/// Setup:
///   repo_a: mirror SHA == local HEAD → up-to-date → loop continues
///   repo_b: mirror SHA != local HEAD → update detected → check_self_update runs
///
/// Observable: check_self_update pulls the new commit and executes build_cmd,
/// which writes a canary file. If the bug were present (return instead of
/// continue), the loop would exit at repo_a and the canary would never appear.
#[tokio::test]
async fn check_remote_updates_continues_past_up_to_date_repo() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let mirrors_dir = data_dir.join("mirrors");
    std::fs::create_dir_all(&mirrors_dir).expect("create mirrors");

    let repo_a_path = setup_uptodate_repo(tmp.path(), &mirrors_dir);
    let repo_b_path = setup_behind_repo(tmp.path(), &mirrors_dir);

    let canary = data_dir.join("build_ran");
    let build_cmd = format!("touch {}", canary.to_str().expect("canary path"));

    let config = make_config(
        data_dir.to_str().expect("data_dir"),
        vec![self_repo(&repo_a_path), self_repo(&repo_b_path)],
        build_cmd,
    );

    let mut db = Db::open(":memory:").expect("open db");
    db.migrate().expect("migrate");
    let db = Arc::new(db);

    let (pipeline, _rx) = Pipeline::new(
        db,
        HashMap::new(),
        config,
        SandboxMode::Direct,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        false,
    );

    pipeline.check_remote_updates().await;

    assert!(
        canary.exists(),
        "repo_b update was not detected — the loop likely returned early at repo_a \
         instead of continuing; ensure all guard clauses inside check_remote_updates \
         use `continue` rather than `return`"
    );
}

/// Guard clause: when rev_parse_head fails for the first repo (no commits),
/// the loop continues to the second repo rather than exiting the function.
///
/// Setup:
///   repo_a: valid dir, mirror exists, but rev_parse_head fails (no commits)
///   repo_b: valid dir, mirror exists with commit ahead of local → update detected
#[tokio::test]
async fn check_remote_updates_continues_after_rev_parse_failure() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let mirrors_dir = data_dir.join("mirrors");
    std::fs::create_dir_all(&mirrors_dir).expect("create mirrors");

    // repo_a: initialized but no commits → rev_parse_head fails
    let repo_a = tmp.path().join("repo_a_nocommit");
    std::fs::create_dir_all(&repo_a).expect("create repo_a");
    let repo_a_str = repo_a.to_str().expect("path");
    git_in(repo_a_str, &["init"]);
    // Create a mirror for repo_a so the fetch_origin path isn't taken
    // The mirror has no HEAD either (empty bare repo), so rev_parse returns None
    // → the `let Some(remote) = remote else { continue }` guard fires
    let mirror_a = mirrors_dir.join("repo_a_nocommit.git");
    git_run(&["init", "--bare", mirror_a.to_str().expect("path")]);

    // repo_b: behind its mirror (same as the primary test)
    let repo_b_path = setup_behind_repo(tmp.path(), &mirrors_dir);

    let canary = data_dir.join("build_ran2");
    let build_cmd = format!("touch {}", canary.to_str().expect("canary path"));

    let config = make_config(
        data_dir.to_str().expect("data_dir"),
        vec![self_repo(repo_a_str), self_repo(&repo_b_path)],
        build_cmd,
    );

    let mut db = Db::open(":memory:").expect("open db");
    db.migrate().expect("migrate");
    let db = Arc::new(db);

    let (pipeline, _rx) = Pipeline::new(
        db,
        HashMap::new(),
        config,
        SandboxMode::Direct,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        false,
    );

    pipeline.check_remote_updates().await;

    assert!(
        canary.exists(),
        "repo_b update was not detected — the loop likely returned early when \
         repo_a's mirror had no HEAD; guard clauses must use `continue` not `return`"
    );
}
