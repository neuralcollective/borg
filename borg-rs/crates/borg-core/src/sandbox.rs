//! Process sandbox for pipeline agent phases.
//!
//! Supports two isolation backends (preferred order when `auto`):
//! 1. **bwrap** — bubblewrap-based namespace isolation (no daemon, no image).
//!    Adapted from OpenAI Codex linux-sandbox. Mounts the host filesystem
//!    read-only with selective read-write bind mounts for working dirs.
//! 2. **docker** — Docker container via `docker run`.
//!
//! Set `SANDBOX_BACKEND=auto|bwrap|docker|none` in the environment.
//! Default is `auto` (bwrap if available, else docker, else direct).

use std::{path::Path, process::Stdio};

use tokio::process::Command;
use tracing::{info, warn};

/// Which sandboxing backend to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxMode {
    /// bwrap (bubblewrap) namespace sandbox — lightweight, no daemon required.
    Bwrap,
    /// Docker container — requires daemon and a pre-built image.
    Docker,
    /// No sandboxing — run the process directly on the host.
    Direct,
}

impl SandboxMode {
    /// Parse from env/config string. Unknown values fall back to `Auto`
    /// detection logic.
    pub fn from_str_or_auto(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bwrap" => Some(Self::Bwrap),
            "docker" => Some(Self::Docker),
            "none" | "direct" => Some(Self::Direct),
            _ => None, // "auto" or unrecognised → detect at runtime
        }
    }
}

pub struct Sandbox;

impl Sandbox {
    /// Detect the best available sandbox mode given a preference string.
    ///
    /// Preference order when `preferred` is `"auto"` or empty:
    /// docker → bwrap → direct.
    pub async fn detect(preferred: &str) -> SandboxMode {
        if let Some(forced) = SandboxMode::from_str_or_auto(preferred) {
            return forced;
        }
        // auto — prefer Docker (containerised agents get their own clone)
        if Self::docker_available().await {
            info!("sandbox: docker detected, using container sandbox");
            SandboxMode::Docker
        } else if Self::bwrap_available().await {
            info!("sandbox: docker not found, falling back to bwrap");
            SandboxMode::Bwrap
        } else {
            warn!("sandbox: neither docker nor bwrap available, running agents directly (no isolation)");
            SandboxMode::Direct
        }
    }

    pub async fn bwrap_available() -> bool {
        // bwrap relies on Linux namespaces; skip detection on other platforms
        if cfg!(not(target_os = "linux")) {
            return false;
        }
        Command::new("bwrap")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub async fn docker_available() -> bool {
        Command::new("docker")
            .arg("version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    // --- bwrap backend ---

    /// Build bwrap argument list for `command`.
    ///
    /// Mount order (mirrors OpenAI Codex linux-sandbox/bwrap.rs):
    /// 1. `--ro-bind / /`    — read-only root filesystem
    /// 2. `--dev /dev`       — minimal device tree (null, random, urandom, tty)
    /// 3. `--bind X X`       — per writable_dir (repo, session dir)
    /// 4. `--bind /tmp /tmp` — shared /tmp (needed by compilers, git, etc.)
    /// 5. `--unshare-pid`    — isolated PID namespace
    /// 6. `--new-session`    — setsid (detach terminal)
    /// 7. `--die-with-parent`— auto-cleanup
    /// 8. `--proc /proc`     — fresh procfs for PID namespace
    /// 9. `--chdir`          — working directory inside sandbox
    /// 10. `--`              — command separator
    pub fn bwrap_args(
        writable_dirs: &[&str],
        working_dir: &str,
        command: &[String],
    ) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        args.extend(["--ro-bind", "/", "/", "--dev", "/dev"].map(str::to_string));

        for dir in writable_dirs {
            if !Path::new(dir).exists() {
                warn!("sandbox: skipping non-existent writable dir: {dir}");
                continue;
            }
            args.extend(["--bind", dir, dir].map(str::to_string));
        }

        args.extend(["--bind", "/tmp", "/tmp"].map(str::to_string));

        args.extend(
            [
                "--unshare-pid",
                "--new-session",
                "--die-with-parent",
                "--proc",
                "/proc",
            ]
            .map(str::to_string),
        );

        args.extend(["--chdir", working_dir].map(str::to_string));

        args.push("--".into());
        args.extend_from_slice(command);

        args
    }

    /// Return a `Command` that runs `command` inside a bwrap sandbox.
    ///
    /// Env vars set on the returned `Command` are inherited by the sandboxed
    /// process (bwrap passes them through by default).
    pub fn bwrap_command(writable_dirs: &[&str], working_dir: &str, command: &[String]) -> Command {
        let args = Self::bwrap_args(writable_dirs, working_dir, command);
        let mut cmd = Command::new("bwrap");
        cmd.args(args);
        cmd
    }

    // --- docker backend ---

    /// Return a `Command` that runs inside a Docker container.
    ///
    /// `binds`: `(host_path, container_path, read_only)` — bind mounts.
    /// `volumes`: `(volume_name, container_path)` — named Docker volumes.
    /// `env_vars`: passed as `-e KEY=VALUE` pairs.
    /// `working_dir`: container working directory; skipped if empty.
    /// `command`: appended after the image name (empty = use entrypoint default).
    /// `memory_mb`: memory limit in MiB (0 = no limit).
    /// `cpus`: CPU quota (0.0 = no limit).
    /// `network`: bridge network name. `Some(name)` uses that network + Google DNS;
    ///            `None` falls back to `--network host`.
    pub fn docker_command(
        image: &str,
        binds: &[(&str, &str, bool)],
        volumes: &[(&str, &str)],
        working_dir: &str,
        command: &[String],
        env_vars: &[(&str, &str)],
        memory_mb: u64,
        cpus: f64,
        network: Option<&str>,
    ) -> Command {
        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(),
            "-i".to_string(),
            "--pids-limit".to_string(),
            "256".to_string(),
            "--label".to_string(),
            "borg-agent=1".to_string(),
        ];

        if memory_mb > 0 {
            args.push("--memory".to_string());
            args.push(format!("{memory_mb}m"));
        }
        if cpus > 0.0 {
            args.push("--cpus".to_string());
            args.push(format!("{cpus:.2}"));
        }

        // Linux-only security hardening and networking
        if cfg!(target_os = "linux") {
            args.extend([
                "--security-opt", "no-new-privileges:true",
                "--cap-drop", "ALL",
            ].map(str::to_string));
            if let Some(net) = network {
                args.extend(["--network".to_string(), net.to_string()]);
                args.extend(["--dns".to_string(), "8.8.8.8".to_string()]);
                args.extend(["--dns".to_string(), "8.8.4.4".to_string()]);
            } else {
                args.extend(["--network".to_string(), "host".to_string()]);
            }
        }

        for (host, container, ro) in binds {
            args.push("-v".to_string());
            if *ro {
                args.push(format!("{host}:{container}:ro"));
            } else {
                args.push(format!("{host}:{container}"));
            }
        }

        for (name, container) in volumes {
            args.push("-v".to_string());
            args.push(format!("{name}:{container}"));
        }

        for (key, val) in env_vars {
            args.push("-e".to_string());
            args.push(format!("{key}={val}"));
        }

        if !working_dir.is_empty() {
            args.push("-w".to_string());
            args.push(working_dir.to_string());
        }

        args.push(image.to_string());
        args.extend_from_slice(command);

        let mut cmd = Command::new("docker");
        cmd.args(args);
        cmd
    }

    /// Network name used for agent containers.
    pub const AGENT_NETWORK: &'static str = "borg-agent-net";

    /// Subnet for the agent bridge network.
    pub const AGENT_SUBNET: &'static str = "172.30.0.0/16";

    /// Create the borg-agent-net bridge network if it doesn't already exist.
    /// Returns true if the network is available (created or already existed).
    pub async fn ensure_agent_network() -> bool {
        let exists = tokio::process::Command::new("docker")
            .args(["network", "inspect", Self::AGENT_NETWORK])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if exists {
            info!("sandbox: agent network {} already exists", Self::AGENT_NETWORK);
            return true;
        }

        let ok = tokio::process::Command::new("docker")
            .args([
                "network", "create",
                "--driver", "bridge",
                "--subnet", Self::AGENT_SUBNET,
                Self::AGENT_NETWORK,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if ok {
            info!("sandbox: created agent network {}", Self::AGENT_NETWORK);
        } else {
            warn!("sandbox: failed to create agent network {}", Self::AGENT_NETWORK);
        }
        ok
    }

    /// Install iptables rules that block agent containers from reaching localhost and LAN.
    /// Rules are inserted into the DOCKER-USER chain (Docker's designated chain for user rules).
    /// This is idempotent — rules are checked before insertion.
    pub async fn install_network_rules() -> bool {
        if cfg!(not(target_os = "linux")) {
            return false;
        }

        // (source, dest, action) — order matters: ACCEPT for 172.30/16 before DROP for 172.16/12
        let rules: &[(&str, &str, &str)] = &[
            (Self::AGENT_SUBNET, "172.30.0.0/16", "ACCEPT"),
            (Self::AGENT_SUBNET, "127.0.0.0/8",   "DROP"),
            (Self::AGENT_SUBNET, "10.0.0.0/8",    "DROP"),
            (Self::AGENT_SUBNET, "192.168.0.0/16", "DROP"),
            (Self::AGENT_SUBNET, "169.254.0.0/16", "DROP"),
            (Self::AGENT_SUBNET, "172.16.0.0/12",  "DROP"),
        ];

        let mut all_ok = true;
        for (src, dst, action) in rules {
            // Check if rule already exists
            let exists = tokio::process::Command::new("iptables")
                .args(["-C", "DOCKER-USER", "-s", src, "-d", dst, "-j", action])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);

            if exists {
                continue;
            }

            let ok = tokio::process::Command::new("iptables")
                .args(["-I", "DOCKER-USER", "-s", src, "-d", dst, "-j", action])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);

            if ok {
                info!("sandbox: iptables DOCKER-USER -s {src} -d {dst} -j {action}");
            } else {
                warn!("sandbox: failed to install iptables rule -s {src} -d {dst} -j {action} (needs CAP_NET_ADMIN?)");
                all_ok = false;
            }
        }
        all_ok
    }

    /// Remove the iptables rules installed by install_network_rules().
    pub async fn remove_network_rules() {
        if cfg!(not(target_os = "linux")) {
            return;
        }

        let rules: &[(&str, &str, &str)] = &[
            (Self::AGENT_SUBNET, "172.30.0.0/16", "ACCEPT"),
            (Self::AGENT_SUBNET, "127.0.0.0/8",   "DROP"),
            (Self::AGENT_SUBNET, "10.0.0.0/8",    "DROP"),
            (Self::AGENT_SUBNET, "192.168.0.0/16", "DROP"),
            (Self::AGENT_SUBNET, "169.254.0.0/16", "DROP"),
            (Self::AGENT_SUBNET, "172.16.0.0/12",  "DROP"),
        ];

        for (src, dst, action) in rules {
            let _ = tokio::process::Command::new("iptables")
                .args(["-D", "DOCKER-USER", "-s", src, "-d", dst, "-j", action])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
        info!("sandbox: removed agent network iptables rules");
    }

    /// Remove the agent network (best-effort, called on shutdown).
    pub async fn remove_agent_network() {
        let _ = tokio::process::Command::new("docker")
            .args(["network", "rm", Self::AGENT_NETWORK])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        info!("sandbox: removed agent network {}", Self::AGENT_NETWORK);
    }

    /// Remove any containers with label `borg-agent=1` that are not running.
    /// Call once at startup to clean up orphans from a previous crash.
    pub async fn prune_orphan_containers() {
        let Ok(out) = tokio::process::Command::new("docker")
            .args([
                "ps", "-a", "--filter", "label=borg-agent=1",
                "--filter", "status=exited",
                "--filter", "status=dead",
                "--filter", "status=created",
                "--format", "{{.ID}}",
            ])
            .output()
            .await
        else {
            return;
        };
        let stdout = String::from_utf8_lossy(&out.stdout);
        let ids: Vec<&str> = stdout
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if ids.is_empty() {
            return;
        }
        let mut cmd = tokio::process::Command::new("docker");
        cmd.arg("rm").arg("-f");
        cmd.args(&ids);
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if let Ok(status) = cmd.status().await {
            if status.success() {
                info!("pruned {} orphan borg-agent container(s)", ids.len());
            }
        }
    }

    /// Compute a short 8-char hex hash of a branch name for stable volume naming.
    /// Uses FNV-1a so there's no external crate dependency.
    pub fn branch_hash(branch: &str) -> String {
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in branch.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
            .chars()
            .take(8)
            .collect()
    }

    /// Return the per-branch cache volume name for a given repo + branch + cache type.
    /// E.g. `borg-cache-myrepo-a1b2c3d4-target`.
    pub fn branch_volume_name(repo: &str, branch: &str, cache_type: &str) -> String {
        let h = Self::branch_hash(branch);
        format!("borg-cache-{repo}-{h}-{cache_type}")
    }

    /// Return the `main`-branch cache volume name (used as warm seed).
    pub fn main_volume_name(repo: &str, cache_type: &str) -> String {
        Self::branch_volume_name(repo, "main", cache_type)
    }

    /// Ensure a Docker volume exists with the given name and label it with
    /// the current unix timestamp as `borg-last-used`.
    pub async fn touch_volume(name: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // `docker volume create` is idempotent — no-op if already exists.
        let _ = tokio::process::Command::new("docker")
            .args([
                "volume", "create",
                "--label", &format!("borg-last-used={now}"),
                name,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
    }

    /// If a `main` cache volume exists for the repo and the branch volume does
    /// not yet exist, copy the main cache into the new branch volume so the
    /// first build is warm. No-op if branch volume already exists or main
    /// volume is absent.
    pub async fn warm_branch_cache(repo: &str, branch: &str, cache_type: &str, helper_image: &str) {
        let main_vol = Self::main_volume_name(repo, cache_type);
        let branch_vol = Self::branch_volume_name(repo, branch, cache_type);

        if branch_vol == main_vol {
            // branch IS main — nothing to warm
            Self::touch_volume(&main_vol).await;
            return;
        }

        // Check if branch volume already exists (avoid redundant copy)
        let exists = tokio::process::Command::new("docker")
            .args(["volume", "inspect", &branch_vol])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if exists {
            Self::touch_volume(&branch_vol).await;
            return;
        }

        // Check if main cache volume exists
        let main_exists = tokio::process::Command::new("docker")
            .args(["volume", "inspect", &main_vol])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        Self::touch_volume(&branch_vol).await;

        if main_exists {
            // Copy main → branch using a minimal busybox container.
            // Mount both volumes and rsync/cp the contents.
            let status = tokio::process::Command::new("docker")
                .args([
                    "run", "--rm",
                    "-v", &format!("{main_vol}:/src:ro"),
                    "-v", &format!("{branch_vol}:/dst"),
                    helper_image,
                    "sh", "-c", "cp -a /src/. /dst/ 2>/dev/null || true",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            match status {
                Ok(s) if s.success() => {
                    info!(repo, branch, cache_type, "warmed branch cache from main");
                }
                _ => {
                    info!(repo, branch, cache_type, "cache warm skipped (copy failed or no main cache)");
                }
            }
        } else {
            info!(repo, branch, cache_type, "no main cache to warm from — starting cold");
        }
    }

    /// Remove stale borg-cache volumes not used in the last `max_age_days` days.
    /// "Last used" is read from the `borg-last-used` label written by `touch_volume`.
    /// Falls back to Docker's volume CreatedAt if the label is absent.
    pub async fn prune_stale_cache_volumes(max_age_days: u64) {
        let threshold_secs = max_age_days * 86_400;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let Ok(out) = tokio::process::Command::new("docker")
            .args([
                "volume", "ls",
                "--filter", "name=borg-cache-",
                "--format", "{{.Name}}",
            ])
            .output()
            .await
        else {
            return;
        };

        let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();

        for name in names {
            let last_used = Self::volume_last_used(&name).await;
            if let Some(ts) = last_used {
                if now.saturating_sub(ts) > threshold_secs {
                    if Self::remove_volume(&name).await {
                        info!("evicted stale cache volume: {name} (last used {ts})");
                    }
                }
            }
        }
    }

    /// Read the `borg-last-used` label from a volume, returning unix seconds.
    async fn volume_last_used(name: &str) -> Option<u64> {
        let out = tokio::process::Command::new("docker")
            .args([
                "volume", "inspect", name,
                "--format", "{{index .Labels \"borg-last-used\"}}",
            ])
            .output()
            .await
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        s.trim().parse::<u64>().ok()
    }

    /// List all Docker volumes whose names start with the given prefix.
    /// Returns `(name, size_bytes, last_used_secs)` triples.
    pub async fn list_cache_volumes(prefix: &str) -> Vec<(String, Option<u64>, Option<u64>)> {
        let Ok(out) = tokio::process::Command::new("docker")
            .args(["volume", "ls", "--filter", &format!("name={prefix}"), "--format", "{{.Name}}"])
            .output()
            .await
        else {
            return vec![];
        };
        let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();

        let size_map = Self::volume_sizes().await;

        let mut result = Vec::with_capacity(names.len());
        for n in names {
            let sz = size_map.get(&n).copied();
            let last_used = Self::volume_last_used(&n).await;
            result.push((n, sz, last_used));
        }
        result
    }

    async fn volume_sizes() -> std::collections::HashMap<String, u64> {
        let Ok(out) = tokio::process::Command::new("docker")
            .args(["system", "df", "-v", "--format", "{{json .}}"])
            .output()
            .await
        else {
            return Default::default();
        };

        // `docker system df -v --format '{{json .}}'` emits one JSON object per entity.
        // Volume entries have {"Name":"...","Size":"1.2GB",...}.
        let mut map = std::collections::HashMap::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(name) = v.get("Name").and_then(|n| n.as_str()) else {
                continue;
            };
            let Some(size_str) = v.get("Size").and_then(|s| s.as_str()) else {
                continue;
            };
            if let Some(bytes) = parse_docker_size(size_str) {
                map.insert(name.to_string(), bytes);
            }
        }
        map
    }

    /// Remove a named Docker volume. Returns true if successful.
    pub async fn remove_volume(name: &str) -> bool {
        tokio::process::Command::new("docker")
            .args(["volume", "rm", name])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn parse_docker_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, unit) = if let Some(n) = s.strip_suffix("GB") {
        (n.trim().parse::<f64>().ok()?, 1_000_000_000u64)
    } else if let Some(n) = s.strip_suffix("MB") {
        (n.trim().parse::<f64>().ok()?, 1_000_000u64)
    } else if let Some(n) = s.strip_suffix("kB") {
        (n.trim().parse::<f64>().ok()?, 1_000u64)
    } else if let Some(n) = s.strip_suffix("B") {
        (n.trim().parse::<f64>().ok()?, 1u64)
    } else {
        return None;
    };
    Some((num * unit as f64) as u64)
}

#[cfg(test)]
mod tests {
    use super::parse_docker_size;

    #[test]
    fn fractional_gb() {
        assert_eq!(parse_docker_size("1.5GB"), Some(1_500_000_000));
    }

    #[test]
    fn integer_mb() {
        assert_eq!(parse_docker_size("512MB"), Some(512_000_000));
    }

    #[test]
    fn kilobytes() {
        assert_eq!(parse_docker_size("100kB"), Some(100_000));
    }

    #[test]
    fn bare_bytes() {
        assert_eq!(parse_docker_size("256B"), Some(256));
    }

    #[test]
    fn unknown_suffix_returns_none() {
        assert_eq!(parse_docker_size("1TB"), None);
    }

    #[test]
    fn whitespace_padded() {
        assert_eq!(parse_docker_size("  2.0GB  "), Some(2_000_000_000));
    }

    #[test]
    fn empty_string_returns_none() {
        assert_eq!(parse_docker_size(""), None);
    }
}
