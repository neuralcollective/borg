use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, warn};

pub struct DrainConfig<'a> {
    pub backend: &'a str,
    pub task_id: i64,
    pub phase_name: &'a str,
    pub timeout_s: u64,
    pub stream_tx: Option<UnboundedSender<String>>,
    /// Returns true if the stderr line is fatal (will set had_fatal_stderr).
    pub is_warning_stderr: fn(&str) -> bool,
}

pub struct DrainResult {
    pub output: String,
    pub timed_out: bool,
    pub had_fatal_stderr: bool,
}

pub async fn drain_child(child: &mut Child, cfg: DrainConfig<'_>) -> Result<DrainResult> {
    let stdout = child.stdout.take().context("failed to take stdout")?;
    let stderr = child.stderr.take().context("failed to take stderr")?;

    let mut output_lines: Vec<String> = Vec::new();
    let mut had_fatal_stderr = false;
    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut timed_out = false;
    let timeout_enabled = cfg.timeout_s > 0;
    let timeout_secs = cfg.timeout_s.max(1);
    let timeout = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs));
    tokio::pin!(timeout);

    let backend = cfg.backend;
    let task_id = cfg.task_id;
    let phase_name = cfg.phase_name;

    while !(stdout_done && stderr_done) {
        tokio::select! {
            _ = &mut timeout, if timeout_enabled => {
                timed_out = true;
                warn!(
                    task_id,
                    phase = %phase_name,
                    timeout_s = timeout_secs,
                    "{backend} phase timed out, terminating subprocess"
                );
                let _ = child.start_kill();
                break;
            }
            line = stdout_reader.next_line(), if !stdout_done => {
                match line.context("error reading stdout")? {
                    Some(l) => {
                        if let Some(tx) = &cfg.stream_tx {
                            let _ = tx.send(l.clone());
                        }
                        if output_lines.len() < 50_000 {
                            output_lines.push(l);
                        }
                    }
                    None => {
                        stdout_done = true;
                    }
                }
            }
            line = stderr_reader.next_line(), if !stderr_done => {
                match line {
                    Ok(Some(l)) => {
                        if !l.is_empty() {
                            if (cfg.is_warning_stderr)(&l) {
                                had_fatal_stderr = true;
                                warn!(task_id, phase = %phase_name, "{backend} stderr: {}", l);
                            } else {
                                debug!(task_id, phase = %phase_name, "{backend} stderr: {}", l);
                            }
                        }
                    }
                    Ok(None) => {
                        stderr_done = true;
                    }
                    Err(e) => {
                        warn!(task_id, phase = %phase_name, "{backend} stderr read error: {e}");
                        stderr_done = true;
                    }
                }
            }
        }
    }

    Ok(DrainResult {
        output: output_lines.join("\n"),
        timed_out,
        had_fatal_stderr,
    })
}
