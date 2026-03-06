import { useState } from "react";
import { useAuth } from "@/lib/auth";
import { BorgLogo } from "./borg-logo";

export function LoginPage() {
  const { needsSetup, login, setup } = useAuth();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (busy || !username.trim() || !password) return;
    setBusy(true);
    setError("");

    const err = needsSetup
      ? await setup(username.trim(), password, displayName.trim() || undefined)
      : await login(username.trim(), password);

    if (err) setError(err);
    setBusy(false);
  }

  return (
    <div className="flex h-screen items-center justify-center bg-[#09090b]">
      <div className="w-full max-w-sm space-y-6 px-6">
        <div className="flex flex-col items-center gap-3">
          <div className="h-14 w-14">
            <BorgLogo expanded />
          </div>
          <h1 className="text-lg font-semibold text-zinc-200">
            {needsSetup ? "Create Admin Account" : "Sign In"}
          </h1>
          {needsSetup && (
            <p className="text-center text-[12px] text-zinc-500">
              No users exist yet. Create the first admin account to get started.
            </p>
          )}
        </div>

        <form onSubmit={handleSubmit} className="space-y-3">
          <div>
            <label className="mb-1 block text-[11px] text-zinc-500">Username</label>
            <input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoFocus
              className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 outline-none focus:border-blue-500/40"
              placeholder="admin"
            />
          </div>

          {needsSetup && (
            <div>
              <label className="mb-1 block text-[11px] text-zinc-500">Display Name</label>
              <input
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 outline-none focus:border-blue-500/40"
                placeholder="Your Name"
              />
            </div>
          )}

          <div>
            <label className="mb-1 block text-[11px] text-zinc-500">Password</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 outline-none focus:border-blue-500/40"
              placeholder={needsSetup ? "Min 4 characters" : "••••••"}
            />
          </div>

          {error && (
            <div className="rounded-md border border-red-500/20 bg-red-500/[0.07] px-3 py-2 text-[12px] text-red-400">
              {error}
            </div>
          )}

          <button
            type="submit"
            disabled={busy || !username.trim() || !password}
            className="w-full rounded-md bg-blue-500/20 py-2 text-[13px] font-medium text-blue-400 ring-1 ring-inset ring-blue-500/20 transition-colors hover:bg-blue-500/30 disabled:opacity-50"
          >
            {busy ? "..." : needsSetup ? "Create Account" : "Sign In"}
          </button>
        </form>
      </div>
    </div>
  );
}
