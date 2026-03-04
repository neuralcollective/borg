import { test, expect, afterAll } from 'bun:test';
import { spawn } from 'child_process';
import { killWithFallback, waitForExit } from './process-utils.js';

// ── killWithFallback unit tests (mock process objects) ───────────────────

test('killWithFallback sends SIGTERM immediately', () => {
  const signals = [];
  const proc = { exitCode: null, kill(sig) { signals.push(sig); } };
  killWithFallback(proc, 100);
  expect(signals).toEqual(['SIGTERM']);
});

test('killWithFallback sends SIGKILL after timeout when process has not exited', async () => {
  const signals = [];
  const proc = { exitCode: null, kill(sig) { signals.push(sig); } };
  killWithFallback(proc, 50);
  await new Promise(r => setTimeout(r, 120));
  expect(signals).toEqual(['SIGTERM', 'SIGKILL']);
});

test('killWithFallback does not send SIGKILL when process exits before timeout', async () => {
  const signals = [];
  const proc = {
    exitCode: null,
    kill(sig) {
      signals.push(sig);
      if (sig === 'SIGTERM') this.exitCode = 0;
    },
  };
  killWithFallback(proc, 50);
  await new Promise(r => setTimeout(r, 120));
  expect(signals).toEqual(['SIGTERM']);
});

test('killWithFallback does nothing when process has already exited', () => {
  const signals = [];
  const proc = { exitCode: 0, kill(sig) { signals.push(sig); } };
  killWithFallback(proc, 50);
  expect(signals).toEqual([]);
});

// ── waitForExit integration tests (real processes) ───────────────────────

test('waitForExit resolves when process exits naturally', async () => {
  const proc = spawn('sh', ['-c', 'exit 0']);
  const start = Date.now();
  await waitForExit(proc, 2000);
  expect(Date.now() - start).toBeLessThan(2000);
  expect(proc.exitCode).toBe(0);
});

test('waitForExit resolves after timeout when process does not exit', async () => {
  const proc = spawn('sleep', ['60']);
  const start = Date.now();
  await waitForExit(proc, 100);
  const elapsed = Date.now() - start;
  expect(elapsed).toBeGreaterThanOrEqual(100);
  proc.kill('SIGKILL');
});

test('waitForExit resolves immediately when process has already exited', async () => {
  const proc = spawn('sh', ['-c', 'exit 0']);
  // wait for it to finish
  await new Promise(r => proc.once('close', r));
  const start = Date.now();
  await waitForExit(proc, 2000);
  expect(Date.now() - start).toBeLessThan(50);
});

// ── End-to-end: SIGKILL fallback on a real SIGTERM-ignoring process ───────

test('killWithFallback force-kills a process that ignores SIGTERM', async () => {
  // A shell process that traps SIGTERM and does nothing (ignores it)
  const proc = spawn('sh', ['-c', "trap '' TERM; sleep 60"]);

  // Give the shell a moment to install the trap
  await new Promise(r => setTimeout(r, 100));

  killWithFallback(proc, 200);

  // After SIGTERM + timeout + SIGKILL the process should be dead
  await waitForExit(proc, 1000);

  expect(proc.exitCode !== null || proc.signalCode !== null).toBe(true);
});
