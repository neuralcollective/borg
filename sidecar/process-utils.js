// Helpers for graceful subprocess termination.

const KILL_TIMEOUT_MS = 5000;

/**
 * Send SIGTERM; if the process hasn't exited within timeoutMs, send SIGKILL.
 * The fallback timer is unref'd so it doesn't prevent the event loop from
 * exiting naturally when nothing else is running.
 */
export function killWithFallback(proc, timeoutMs = KILL_TIMEOUT_MS) {
  if (proc.exitCode !== null) return;
  proc.kill('SIGTERM');
  const timer = setTimeout(() => {
    if (proc.exitCode === null) proc.kill('SIGKILL');
  }, timeoutMs);
  timer.unref();
}

/**
 * Resolve when the process emits 'close', or after timeoutMs, whichever comes first.
 */
export function waitForExit(proc, timeoutMs = KILL_TIMEOUT_MS + 1000) {
  if (proc.exitCode !== null) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, timeoutMs);
    proc.once('close', () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

export { KILL_TIMEOUT_MS };
