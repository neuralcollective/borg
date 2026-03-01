use std::collections::HashMap;

use anyhow::Result;

use crate::{db::Db, types::RepoConfig};

/// Full application configuration.
/// Non-sensitive fields are seeded to and loaded from the DB `config` table.
/// Sensitive fields (tokens, API keys) come from env/.env only.
#[derive(Debug, Clone)]
pub struct Config {
    pub telegram_token: String,
    pub oauth_token: String,
    pub assistant_name: String,
    pub trigger_pattern: String,
    pub data_dir: String,
    pub container_image: String,
    pub model: String,
    pub credentials_path: String,
    pub session_max_age_hours: i64,
    pub max_consecutive_errors: u32,

    // Pipeline
    pub pipeline_repo: String,
    pub pipeline_test_cmd: String,
    pub pipeline_lint_cmd: String,
    pub backend: String,
    pub pipeline_admin_chat: String,
    pub release_interval_mins: u32,
    pub continuous_mode: bool,

    // Agent lifecycle
    pub chat_collection_window_ms: i64,
    pub chat_cooldown_ms: i64,
    pub agent_timeout_s: i64,
    pub max_chat_agents: u32,
    pub chat_rate_limit: u32,
    pub pipeline_max_agents: u32,

    // Web dashboard
    pub web_bind: String,
    pub web_port: u16,
    pub dashboard_dist_dir: String,

    // Container / sandbox
    pub container_setup: String,
    pub container_memory_mb: u64,
    /// "auto" (default), "bwrap", "docker", or "none".
    pub sandbox_backend: String,

    // Pipeline tuning
    pub pipeline_max_backlog: u32,
    pub pipeline_seed_cooldown_s: i64,
    pub proposal_promote_threshold: i64,
    pub pipeline_tick_s: u64,
    pub remote_check_interval_s: i64,
    /// Min seconds between automated agent spawns per task (default 120).
    pub pipeline_agent_cooldown_s: i64,

    // Git attribution
    pub git_author_name: String,
    pub git_author_email: String,
    pub git_committer_name: String,
    pub git_committer_email: String,
    pub git_via_borg: bool,
    /// When false (default), tell agent not to add Co-Authored-By Claude trailers.
    pub git_claude_coauthor: bool,
    /// If set, append Co-Authored-By: <value> to every pipeline commit.
    pub git_user_coauthor: String,

    pub watched_repos: Vec<RepoConfig>,

    // Build / self-update
    /// Command to rebuild the binary on self-update (configurable via DB key "build_cmd").
    pub build_cmd: String,

    // Codex
    pub codex_api_key: String,
    pub codex_credentials_path: String,

    // Sidecar (Discord + WhatsApp)
    pub discord_token: String,
    pub wa_auth_dir: String,
    pub wa_disabled: bool,

    // Observer
    pub observer_config: String,
}

fn parse_dotenv() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(contents) = std::fs::read_to_string(".env") else {
        return map;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

fn get(key: &str, dotenv: &HashMap<String, String>) -> Option<String> {
    std::env::var(key).ok().or_else(|| dotenv.get(key).cloned())
}

fn get_str(key: &str, dotenv: &HashMap<String, String>, default: &str) -> String {
    get(key, dotenv).unwrap_or_else(|| default.to_string())
}

fn get_bool(key: &str, dotenv: &HashMap<String, String>, default: bool) -> bool {
    match get(key, dotenv).as_deref() {
        Some("true") | Some("1") => true,
        Some("false") | Some("0") => false,
        Some(_) => default,
        None => default,
    }
}

fn get_i64(key: &str, dotenv: &HashMap<String, String>, default: i64) -> i64 {
    get(key, dotenv)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn get_u32(key: &str, dotenv: &HashMap<String, String>, default: u32) -> u32 {
    get(key, dotenv)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn get_u64(key: &str, dotenv: &HashMap<String, String>, default: u64) -> u64 {
    get(key, dotenv)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn get_u16(key: &str, dotenv: &HashMap<String, String>, default: u16) -> u16 {
    get(key, dotenv)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn resolve_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, path.strip_prefix("~/").unwrap_or(path));
        }
    }
    path.to_string()
}

pub fn codex_has_credentials(path: &str) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return false;
    };
    v.get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|t| t.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

pub fn read_oauth_from_credentials(path: &str) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&contents).ok()?;
    // Try claudeAiOauth.accessToken first, then oauthToken at root
    v.get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .or_else(|| {
            v.get("oauthToken")
                .and_then(|t| t.as_str())
                .map(str::to_string)
        })
}

fn read_oauth_expiry(path: &str) -> Option<i64> {
    let contents = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&contents).ok()?;
    v.get("claudeAiOauth")
        .and_then(|o| o.get("expiresAt"))
        .and_then(|e| e.as_i64())
}

/// Refresh the OAuth token if expired or expiring within 5 minutes.
/// Runs `claude auth status` to trigger CLI-side refresh, then re-reads from disk.
/// Returns the freshest available token, falling back to `current` if all else fails.
pub fn refresh_oauth_token(credentials_path: &str, current: &str) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let expiry = read_oauth_expiry(credentials_path).unwrap_or(0);
    if expiry > 0 && expiry < now_ms + 300_000 {
        tracing::info!("OAuth token expired or near-expiry, refreshing via CLI");
        let _ = std::process::Command::new("claude")
            .args(["auth", "status"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    read_oauth_from_credentials(credentials_path)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| current.to_string())
}

fn parse_watched_repos(
    watched_raw: &str,
    pipeline_repo: &str,
    pipeline_test_cmd: &str,
    pipeline_lint_cmd: &str,
    primary_mode: &str,
) -> Vec<RepoConfig> {
    let mut repos: Vec<RepoConfig> = Vec::new();

    // Primary repo first (is_self = true)
    if !pipeline_repo.is_empty() {
        repos.push(RepoConfig {
            path: pipeline_repo.to_string(),
            test_cmd: pipeline_test_cmd.to_string(),
            prompt_file: String::new(),
            mode: primary_mode.to_string(),
            is_self: true,
            auto_merge: true,
            lint_cmd: pipeline_lint_cmd.to_string(),
            backend: String::new(),
        });
    }

    if watched_raw.is_empty() {
        return repos;
    }

    for entry in watched_raw.split('|') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let parts: Vec<&str> = entry.splitn(5, ':').collect();
        if parts.is_empty() {
            continue;
        }
        let path = parts[0].to_string();
        let mut test_cmd = parts.get(1).copied().unwrap_or("").to_string();
        let prompt_file = parts.get(2).copied().unwrap_or("").to_string();
        let mode = parts.get(3).copied().unwrap_or("sweborg").to_string();
        let lint_cmd = parts.get(4).copied().unwrap_or("").to_string();

        let auto_merge = if test_cmd.ends_with("!manual") {
            test_cmd = test_cmd[..test_cmd.len() - "!manual".len()].to_string();
            false
        } else {
            true
        };

        // Skip if this is the same path as the primary repo (already added)
        if path == pipeline_repo {
            continue;
        }

        repos.push(RepoConfig {
            path,
            test_cmd,
            prompt_file,
            mode,
            is_self: false,
            auto_merge,
            lint_cmd,
            backend: String::new(),
        });
    }

    repos
}

impl Config {
    /// System prompt for chat-facing agents (Telegram, Discord, WhatsApp, web).
    pub fn chat_system_prompt(&self) -> String {
        format!(
            "You are {name}, an autonomous AI agent cluster built by Sasha Duke \
             from Neural Collective and open source contributors. \
             You run 24/7, orchestrating engineering pipelines, answering questions, \
             and executing tasks across Telegram, Discord, and WhatsApp.\n\
             Always refer to yourself as {name} (never as Claude or any other name). \
             Always use first-person plural pronouns (we, us, our) — you are a collective, not a single agent. \
             Keep replies concise and direct.",
            name = self.assistant_name,
        )
    }

    /// Write all non-sensitive fields to DB if not already present (first-run seeding).
    pub fn seed_db(&self, db: &Db) -> Result<()> {
        let entries: &[(&str, String)] = &[
            ("assistant_name", self.assistant_name.clone()),
            ("trigger_pattern", self.trigger_pattern.clone()),
            ("data_dir", self.data_dir.clone()),
            ("container_image", self.container_image.clone()),
            ("model", self.model.clone()),
            (
                "session_max_age_hours",
                self.session_max_age_hours.to_string(),
            ),
            (
                "max_consecutive_errors",
                self.max_consecutive_errors.to_string(),
            ),
            ("pipeline_repo", self.pipeline_repo.clone()),
            ("pipeline_test_cmd", self.pipeline_test_cmd.clone()),
            ("pipeline_lint_cmd", self.pipeline_lint_cmd.clone()),
            ("backend", self.backend.clone()),
            ("pipeline_admin_chat", self.pipeline_admin_chat.clone()),
            (
                "release_interval_mins",
                self.release_interval_mins.to_string(),
            ),
            ("continuous_mode", self.continuous_mode.to_string()),
            (
                "chat_collection_window_ms",
                self.chat_collection_window_ms.to_string(),
            ),
            ("chat_cooldown_ms", self.chat_cooldown_ms.to_string()),
            ("agent_timeout_s", self.agent_timeout_s.to_string()),
            ("max_chat_agents", self.max_chat_agents.to_string()),
            ("chat_rate_limit", self.chat_rate_limit.to_string()),
            ("pipeline_max_agents", self.pipeline_max_agents.to_string()),
            ("web_bind", self.web_bind.clone()),
            ("web_port", self.web_port.to_string()),
            ("dashboard_dist_dir", self.dashboard_dist_dir.clone()),
            ("container_setup", self.container_setup.clone()),
            ("container_memory_mb", self.container_memory_mb.to_string()),
            ("sandbox_backend", self.sandbox_backend.clone()),
            (
                "pipeline_max_backlog",
                self.pipeline_max_backlog.to_string(),
            ),
            (
                "pipeline_seed_cooldown_s",
                self.pipeline_seed_cooldown_s.to_string(),
            ),
            (
                "proposal_promote_threshold",
                self.proposal_promote_threshold.to_string(),
            ),
            ("pipeline_tick_s", self.pipeline_tick_s.to_string()),
            (
                "remote_check_interval_s",
                self.remote_check_interval_s.to_string(),
            ),
            ("git_author_name", self.git_author_name.clone()),
            ("git_author_email", self.git_author_email.clone()),
            ("git_committer_name", self.git_committer_name.clone()),
            ("git_committer_email", self.git_committer_email.clone()),
            ("git_via_borg", self.git_via_borg.to_string()),
            ("git_claude_coauthor", self.git_claude_coauthor.to_string()),
            ("git_user_coauthor", self.git_user_coauthor.clone()),
            ("build_cmd", "cargo build --release".into()),
            ("observer_config", self.observer_config.clone()),
            ("wa_disabled", self.wa_disabled.to_string()),
        ];
        let conn_guard = db.raw_conn();
        let conn = conn_guard.lock().unwrap_or_else(|e| e.into_inner());
        for (key, value) in entries {
            conn.execute(
                "INSERT OR IGNORE INTO config (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
                rusqlite::params![key, value],
            )?;
        }
        Ok(())
    }

    /// Return a new Config with non-sensitive fields overridden from DB values.
    pub fn load_from_db(&self, db: &Db) -> Self {
        let mut c = self.clone();
        let get = |key: &str| db.get_config(key).ok().flatten();
        let get_str = |key: &str, cur: &str| get(key).unwrap_or_else(|| cur.to_string());
        let get_bool =
            |key: &str, cur: bool| get(key).map(|v| v == "true" || v == "1").unwrap_or(cur);
        macro_rules! load_i64 {
            ($key:expr, $field:expr) => {
                if let Some(v) = get($key).and_then(|s| s.parse().ok()) {
                    $field = v;
                }
            };
        }
        macro_rules! load_u32 {
            ($key:expr, $field:expr) => {
                if let Some(v) = get($key).and_then(|s| s.parse().ok()) {
                    $field = v;
                }
            };
        }
        macro_rules! load_u64 {
            ($key:expr, $field:expr) => {
                if let Some(v) = get($key).and_then(|s| s.parse().ok()) {
                    $field = v;
                }
            };
        }
        macro_rules! load_u16 {
            ($key:expr, $field:expr) => {
                if let Some(v) = get($key).and_then(|s| s.parse().ok()) {
                    $field = v;
                }
            };
        }
        c.assistant_name = get_str("assistant_name", &c.assistant_name);
        c.trigger_pattern = get_str("trigger_pattern", &c.trigger_pattern);
        c.data_dir = get_str("data_dir", &c.data_dir);
        c.container_image = get_str("container_image", &c.container_image);
        c.model = get_str("model", &c.model);
        c.backend = get_str("backend", &c.backend);
        c.pipeline_repo = get_str("pipeline_repo", &c.pipeline_repo);
        c.pipeline_test_cmd = get_str("pipeline_test_cmd", &c.pipeline_test_cmd);
        c.pipeline_lint_cmd = get_str("pipeline_lint_cmd", &c.pipeline_lint_cmd);
        c.pipeline_admin_chat = get_str("pipeline_admin_chat", &c.pipeline_admin_chat);
        c.container_setup = get_str("container_setup", &c.container_setup);
        c.sandbox_backend = get_str("sandbox_backend", &c.sandbox_backend);
        c.web_bind = get_str("web_bind", &c.web_bind);
        c.dashboard_dist_dir = get_str("dashboard_dist_dir", &c.dashboard_dist_dir);
        c.git_author_name = get_str("git_author_name", &c.git_author_name);
        c.git_author_email = get_str("git_author_email", &c.git_author_email);
        c.git_committer_name = get_str("git_committer_name", &c.git_committer_name);
        c.git_committer_email = get_str("git_committer_email", &c.git_committer_email);
        c.git_user_coauthor = get_str("git_user_coauthor", &c.git_user_coauthor);
        c.observer_config = get_str("observer_config", &c.observer_config);
        c.build_cmd = get_str("build_cmd", &c.build_cmd);
        c.continuous_mode = get_bool("continuous_mode", c.continuous_mode);
        c.git_via_borg = get_bool("git_via_borg", c.git_via_borg);
        c.git_claude_coauthor = get_bool("git_claude_coauthor", c.git_claude_coauthor);
        c.wa_disabled = get_bool("wa_disabled", c.wa_disabled);
        load_i64!("session_max_age_hours", c.session_max_age_hours);
        load_i64!("chat_collection_window_ms", c.chat_collection_window_ms);
        load_i64!("chat_cooldown_ms", c.chat_cooldown_ms);
        load_i64!("agent_timeout_s", c.agent_timeout_s);
        load_i64!("pipeline_seed_cooldown_s", c.pipeline_seed_cooldown_s);
        load_i64!("proposal_promote_threshold", c.proposal_promote_threshold);
        load_i64!("remote_check_interval_s", c.remote_check_interval_s);
        load_u32!("max_consecutive_errors", c.max_consecutive_errors);
        load_u32!("release_interval_mins", c.release_interval_mins);
        load_u32!("max_chat_agents", c.max_chat_agents);
        load_u32!("chat_rate_limit", c.chat_rate_limit);
        load_u32!("pipeline_max_agents", c.pipeline_max_agents);
        load_u32!("pipeline_max_backlog", c.pipeline_max_backlog);
        load_u64!("container_memory_mb", c.container_memory_mb);
        load_u64!("pipeline_tick_s", c.pipeline_tick_s);
        load_u16!("web_port", c.web_port);
        c
    }

    pub fn from_env() -> Result<Self> {
        let dotenv = parse_dotenv();

        let home = std::env::var("HOME").unwrap_or_default();
        let default_credentials = format!("{}/.claude/.credentials.json", home);
        let default_codex_credentials = format!("{}/.codex/auth.json", home);

        let credentials_path = get_str("CREDENTIALS_PATH", &dotenv, &default_credentials);
        let credentials_path = resolve_tilde(&credentials_path);

        let codex_credentials_path = get_str(
            "CODEX_CREDENTIALS_PATH",
            &dotenv,
            &default_codex_credentials,
        );
        let codex_credentials_path = resolve_tilde(&codex_credentials_path);
        let codex_api_key = get_str("OPENAI_API_KEY", &dotenv, "");

        // OAuth token: env/dotenv first, then credentials file
        let oauth_token = get("CLAUDE_CODE_OAUTH_TOKEN", &dotenv)
            .filter(|s| !s.is_empty())
            .or_else(|| read_oauth_from_credentials(&credentials_path))
            .unwrap_or_default();

        let pipeline_repo = get_str("PIPELINE_REPO", &dotenv, "");
        let pipeline_test_cmd = get_str("PIPELINE_TEST_CMD", &dotenv, "");
        let pipeline_lint_cmd = get_str("PIPELINE_LINT_CMD", &dotenv, "");
        let backend = get_str("BACKEND", &dotenv, "claude");
        let pipeline_mode = get_str("PIPELINE_MODE", &dotenv, "sweborg");
        let watched_raw = get_str("WATCHED_REPOS", &dotenv, "");

        let watched_repos = parse_watched_repos(
            &watched_raw,
            &pipeline_repo,
            &pipeline_test_cmd,
            &pipeline_lint_cmd,
            &pipeline_mode,
        );

        Ok(Config {
            telegram_token: get_str("TELEGRAM_BOT_TOKEN", &dotenv, ""),
            oauth_token,
            assistant_name: get_str("ASSISTANT_NAME", &dotenv, "Borg"),
            trigger_pattern: get_str("TRIGGER_PATTERN", &dotenv, "@Borg"),
            data_dir: get_str("DATA_DIR", &dotenv, "store"),
            container_image: get_str("CONTAINER_IMAGE", &dotenv, "borg-agent"),
            model: get_str("MODEL", &dotenv, "claude-sonnet-4-6"),
            credentials_path,
            session_max_age_hours: get_i64("SESSION_MAX_AGE_HOURS", &dotenv, 24),
            max_consecutive_errors: get_u32("MAX_CONSECUTIVE_ERRORS", &dotenv, 3),
            pipeline_repo,
            pipeline_test_cmd,
            pipeline_lint_cmd,
            backend,
            pipeline_admin_chat: get_str("PIPELINE_ADMIN_CHAT", &dotenv, ""),
            release_interval_mins: get_u32("RELEASE_INTERVAL_MINS", &dotenv, 180),
            continuous_mode: get_bool("CONTINUOUS_MODE", &dotenv, false),
            chat_collection_window_ms: get_i64("CHAT_COLLECTION_WINDOW_MS", &dotenv, 3000),
            chat_cooldown_ms: get_i64("CHAT_COOLDOWN_MS", &dotenv, 5000),
            agent_timeout_s: get_i64("AGENT_TIMEOUT_S", &dotenv, 1000),
            max_chat_agents: get_u32("MAX_CHAT_AGENTS", &dotenv, 4),
            chat_rate_limit: get_u32("CHAT_RATE_LIMIT", &dotenv, 5),
            pipeline_max_agents: get_u32("PIPELINE_MAX_AGENTS", &dotenv, 4),
            web_bind: get_str("WEB_BIND", &dotenv, "127.0.0.1"),
            web_port: get_u16("WEB_PORT", &dotenv, 3131),
            dashboard_dist_dir: get_str("DASHBOARD_DIST_DIR", &dotenv, "dashboard/dist"),
            container_setup: get_str("CONTAINER_SETUP", &dotenv, ""),
            container_memory_mb: get_u64("CONTAINER_MEMORY_MB", &dotenv, 1024),
            sandbox_backend: get_str("SANDBOX_BACKEND", &dotenv, "auto"),
            pipeline_max_backlog: get_u32("PIPELINE_MAX_BACKLOG", &dotenv, 5),
            pipeline_seed_cooldown_s: get_i64("PIPELINE_SEED_COOLDOWN_S", &dotenv, 3600),
            proposal_promote_threshold: get_i64("PIPELINE_PROPOSAL_THRESHOLD", &dotenv, 8),
            pipeline_tick_s: get_u64("PIPELINE_TICK_S", &dotenv, 30),
            remote_check_interval_s: get_i64("REMOTE_CHECK_INTERVAL_S", &dotenv, 300),
            pipeline_agent_cooldown_s: get_i64("PIPELINE_AGENT_COOLDOWN_S", &dotenv, 120),
            git_author_name: get_str("GIT_AUTHOR_NAME", &dotenv, ""),
            git_author_email: get_str("GIT_AUTHOR_EMAIL", &dotenv, ""),
            git_committer_name: get_str("GIT_COMMITTER_NAME", &dotenv, ""),
            git_committer_email: get_str("GIT_COMMITTER_EMAIL", &dotenv, ""),
            git_via_borg: get_bool("GIT_VIA_BORG", &dotenv, false),
            git_claude_coauthor: get_bool("GIT_CLAUDE_COAUTHOR", &dotenv, false),
            git_user_coauthor: get_str("GIT_USER_COAUTHOR", &dotenv, ""),
            watched_repos,
            build_cmd: "cargo build --release".into(),
            codex_api_key,
            codex_credentials_path,
            discord_token: get_str("DISCORD_TOKEN", &dotenv, ""),
            wa_auth_dir: get_str("WA_AUTH_DIR", &dotenv, ""),
            wa_disabled: get_bool("WA_DISABLED", &dotenv, false),
            observer_config: get_str("OBSERVER_CONFIG", &dotenv, ""),
        })
    }
}
