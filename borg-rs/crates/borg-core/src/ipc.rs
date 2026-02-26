use std::io::Read;

use tracing::warn;

/// Hard cap for any file read through the safe IPC channel (1 MiB).
pub const MAX_IPC_FILE_BYTES: u64 = 1 << 20; // 1_048_576

/// Outcome of a safe IPC file check/read.
#[derive(Debug)]
pub enum IpcReadResult {
    /// File contents (guaranteed ≤ MAX_IPC_FILE_BYTES bytes, no symlinks).
    Ok(String),
    /// File does not exist (not an error, callers may fall through).
    NotFound,
    /// File was rejected and moved to the errors/ quarantine dir.
    Quarantined(String),
}

/// Verify that `name` contains no path-traversal segments.
///
/// Returns `Err` if `name` is empty, absolute, or contains a `..` component.
/// Subdirectory separators (e.g. `.borg/prompt.md`) are allowed as long as
/// no component is `..` and the path is relative.
pub fn validate_filename(name: &str) -> anyhow::Result<()> {
    use std::path::Component;
    if name.is_empty() {
        anyhow::bail!("empty filename");
    }
    for comp in std::path::Path::new(name).components() {
        match comp {
            Component::ParentDir => anyhow::bail!("path traversal not allowed: {name}"),
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("absolute path not allowed: {name}")
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read `base_dir/name` with defense-in-depth:
///
/// 1. `validate_filename` — reject traversal / absolute.
/// 2. `lstat` — reject symlinks and non-regular files before opening.
/// 3. Open with `O_NOFOLLOW` — close TOCTOU race window.
/// 4. `fstat` post-open — verify regular file and size ≤ `MAX_IPC_FILE_BYTES`.
/// 5. `take(MAX + 1)` guard — detect growth after fstat.
/// 6. UTF-8 validation — artifact files must be valid UTF-8.
///
/// On any violation the offending path is moved to `base_dir/errors/` and
/// `Quarantined` is returned.
pub fn read_file(base_dir: &str, name: &str) -> IpcReadResult {
    if let Err(e) = validate_filename(name) {
        return IpcReadResult::Quarantined(e.to_string());
    }
    let full_path = format!("{base_dir}/{name}");
    let errors_dir = format!("{base_dir}/errors");
    let basename = path_basename(name);
    secure_read(&full_path, &basename, &errors_dir)
}

/// Convenience wrapper: returns `true` only when `base_dir/artifact` is a
/// regular, non-symlink, within-cap file.  Quarantines on policy violation.
pub fn check_artifact(base_dir: &str, artifact: &str) -> bool {
    matches!(read_file(base_dir, artifact), IpcReadResult::Ok(_))
}

/// Like `read_file` but for operator-configured absolute paths.
///
/// Skips `validate_filename` (the path is trusted), but still enforces
/// symlink rejection, size cap, and UTF-8.  Quarantines to a temp dir.
pub fn read_trusted_path(path: &str) -> IpcReadResult {
    let quarantine_dir = std::env::temp_dir()
        .join("borg-ipc-quarantine")
        .to_string_lossy()
        .into_owned();
    let basename = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());
    secure_read(path, &basename, &quarantine_dir)
}

// ── internals ────────────────────────────────────────────────────────────────

/// Extract the last path component as a plain string (for quarantine names).
fn path_basename(name: &str) -> String {
    std::path::Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.replace('/', "_"))
}

/// Core secure-read implementation shared by `read_file` and `read_trusted_path`.
fn secure_read(full_path: &str, basename: &str, quarantine_dir: &str) -> IpcReadResult {
    // Step 1: lstat — detect symlinks and non-regular files without opening.
    let lstat = match std::fs::symlink_metadata(full_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return IpcReadResult::NotFound,
        Err(e) => {
            warn!("ipc: lstat {full_path}: {e}");
            return IpcReadResult::NotFound;
        }
        Ok(m) => m,
    };

    if lstat.file_type().is_symlink() {
        return do_quarantine(full_path, basename, quarantine_dir, "symlink");
    }
    if !lstat.is_file() {
        return do_quarantine(full_path, basename, quarantine_dir, "non-regular file");
    }
    if lstat.len() > MAX_IPC_FILE_BYTES {
        return do_quarantine(full_path, basename, quarantine_dir, "oversized");
    }

    // Step 2: open with O_NOFOLLOW — rejects a symlink swapped in after lstat.
    let file = match open_nofollow(full_path) {
        Err(e) => {
            warn!("ipc: open {full_path}: {e}");
            return do_quarantine(full_path, basename, quarantine_dir, "open failed (possible symlink race)");
        }
        Ok(f) => f,
    };

    // Step 3: fstat the open fd — second opinion on type and size.
    let fstat = match file.metadata() {
        Err(e) => {
            warn!("ipc: fstat {full_path}: {e}");
            return IpcReadResult::Quarantined("fstat failed".to_string());
        }
        Ok(m) => m,
    };

    if !fstat.is_file() {
        return do_quarantine(full_path, basename, quarantine_dir, "non-regular (post-open fstat)");
    }
    if fstat.len() > MAX_IPC_FILE_BYTES {
        return do_quarantine(full_path, basename, quarantine_dir, "oversized (post-open fstat)");
    }

    // Step 4: read with a take-guard one byte above cap to detect growth.
    let mut buf = Vec::with_capacity(fstat.len() as usize);
    match file.take(MAX_IPC_FILE_BYTES + 1).read_to_end(&mut buf) {
        Err(e) => {
            warn!("ipc: read {full_path}: {e}");
            return IpcReadResult::Quarantined("read error".to_string());
        }
        Ok(n) if n as u64 > MAX_IPC_FILE_BYTES => {
            return do_quarantine(full_path, basename, quarantine_dir, "grew beyond cap during read");
        }
        Ok(_) => {}
    }

    // Step 5: UTF-8 validation.
    match String::from_utf8(buf) {
        Err(_) => do_quarantine(full_path, basename, quarantine_dir, "non-UTF-8 content"),
        Ok(s) => IpcReadResult::Ok(s),
    }
}

/// Open `path` refusing to follow symlinks (O_NOFOLLOW).
///
/// On non-Unix targets the flag is unavailable; we fall back to a plain open
/// and log a warning that the TOCTOU window remains open.
#[cfg(unix)]
fn open_nofollow(path: &str) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_nofollow(path: &str) -> std::io::Result<std::fs::File> {
    warn!("ipc: O_NOFOLLOW unavailable on non-Unix; TOCTOU window remains open for {path}");
    std::fs::File::open(path)
}

/// Move `full_path` to `<quarantine_dir>/<basename>.<unix_ts>[.<counter>]`.
///
/// Guards against `quarantine_dir` itself being a symlink; falls back to
/// `remove_file` when rename fails (e.g. cross-device).
fn do_quarantine(full_path: &str, basename: &str, quarantine_dir: &str, reason: &str) -> IpcReadResult {
    // Ensure quarantine_dir is a real directory, not a symlink.
    match std::fs::symlink_metadata(quarantine_dir) {
        Ok(m) if m.file_type().is_symlink() => {
            warn!("ipc: quarantine dir {quarantine_dir:?} is a symlink — skipping move for {full_path} ({reason})");
            let _ = std::fs::remove_file(full_path);
            return IpcReadResult::Quarantined(reason.to_string());
        }
        Ok(_) => {} // real directory (or other non-symlink entry)
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Err(ce) = std::fs::create_dir_all(quarantine_dir) {
                warn!("ipc: could not create quarantine dir {quarantine_dir}: {ce}");
                let _ = std::fs::remove_file(full_path);
                return IpcReadResult::Quarantined(reason.to_string());
            }
        }
        Err(e) => {
            warn!("ipc: stat quarantine dir {quarantine_dir}: {e}");
            let _ = std::fs::remove_file(full_path);
            return IpcReadResult::Quarantined(reason.to_string());
        }
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut dest = format!("{quarantine_dir}/{basename}.{ts}");
    let mut counter = 0u32;
    while std::path::Path::new(&dest).exists()
        || std::fs::symlink_metadata(&dest).is_ok()
    {
        counter += 1;
        dest = format!("{quarantine_dir}/{basename}.{ts}.{counter}");
    }

    if let Err(e) = std::fs::rename(full_path, &dest) {
        warn!("ipc: rename {full_path:?} → {dest:?}: {e} — attempting remove");
        if let Err(re) = std::fs::remove_file(full_path) {
            warn!("ipc: remove {full_path:?}: {re}");
        }
    }

    IpcReadResult::Quarantined(reason.to_string())
}
