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
      <div className="w-full max-w-[400px] space-y-8 px-6">
        <div className="flex flex-col items-center gap-4">
          <div className="relative h-16 w-16">
            <div className="absolute inset-0 rounded-full bg-blue-500/20 blur-xl" />
            <div className="relative h-16 w-16">
              <BorgLogo expanded />
            </div>
          </div>
          <h1 className="text-xl font-semibold text-zinc-200">
            {needsSetup ? "Create Admin Account" : "Sign In"}
          </h1>
          {needsSetup && (
            <p className="text-center text-[13px] text-zinc-500">
              No users exist yet. Create the first admin account to get started.
            </p>
          )}
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label className="mb-1.5 block text-[12px] text-zinc-500">Username</label>
            <input
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoFocus
              className="w-full rounded-xl border border-white/[0.07] bg-white/[0.04] px-4 py-2.5 text-[14px] text-zinc-200 outline-none focus:border-blue-500/40"
              placeholder="admin"
            />
          </div>

          {needsSetup && (
            <div>
              <label className="mb-1.5 block text-[12px] text-zinc-500">Display Name</label>
              <input
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                className="w-full rounded-xl border border-white/[0.07] bg-white/[0.04] px-4 py-2.5 text-[14px] text-zinc-200 outline-none focus:border-blue-500/40"
                placeholder="Your Name"
              />
            </div>
          )}

          <div>
            <label className="mb-1.5 block text-[12px] text-zinc-500">Password</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="w-full rounded-xl border border-white/[0.07] bg-white/[0.04] px-4 py-2.5 text-[14px] text-zinc-200 outline-none focus:border-blue-500/40"
              placeholder={needsSetup ? "Min 4 characters" : "••••••"}
            />
          </div>

          {error && (
            <div className="rounded-xl border border-red-500/20 bg-red-500/[0.07] px-4 py-2.5 text-[13px] text-red-400">
              {error}
            </div>
          )}

          <button
            type="submit"
            disabled={busy || !username.trim() || !password}
            className="w-full rounded-xl bg-blue-500/20 py-2.5 text-[14px] font-medium text-blue-400 ring-1 ring-inset ring-blue-500/20 transition-colors hover:bg-blue-500/30 disabled:opacity-50"
          >
            {busy ? "..." : needsSetup ? "Create Account" : "Sign In"}
          </button>
        </form>
      </div>
    </div>
  );
}
