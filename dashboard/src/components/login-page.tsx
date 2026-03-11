import { useState } from "react";
import { useAuth } from "@/lib/auth";
import { BorgLogo, PRODUCT_WORD } from "./borg-logo";

export function LoginPage() {
  const { needsSetup, login, setup, loginWithSso, ssoProviders, authError } = useAuth();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [showManual, setShowManual] = useState(false);

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

  function formatAuthError(message: string) {
    return message.replaceAll("_", " ");
  }

  const providerButtons = [
    ssoProviders.includes("google") ? { id: "google" as const, label: "Continue with Google" } : null,
    ssoProviders.includes("microsoft") ? { id: "microsoft" as const, label: "Continue with Microsoft" } : null,
  ].filter((provider): provider is { id: "google" | "microsoft"; label: string } => provider !== null);

  return (
    <div className="flex h-screen items-center justify-center bg-[#0f0e0c]">
      <div className="w-full max-w-[400px] space-y-8 px-6">
        <div className="flex flex-col items-center gap-4">
          <div className="relative h-16 w-16">
            <div className="absolute inset-0 rounded-full bg-amber-500/20 blur-xl" />
            <div className="borg-logo relative h-16 w-16 bg-[#1c1a17] rounded-2xl">
              <BorgLogo expanded />
              <div className="borg-logo-ghost grid grid-cols-2 grid-rows-2" aria-hidden>
                {Array.from(PRODUCT_WORD).map((c, i) => (
                  <span key={i} className="flex items-center justify-center text-[22px]">
                    {c}
                  </span>
                ))}
              </div>
            </div>
          </div>
          <h1 className="text-xl font-semibold text-[#e8e0d4]">{needsSetup ? "Create Admin Account" : "Sign In"}</h1>
          {needsSetup && (
            <p className="text-center text-[13px] text-zinc-500">
              No users exist yet. Create the first admin account to get started.
            </p>
          )}
        </div>

        {providerButtons.length > 0 && (
          <div className="space-y-3">
            {providerButtons.map((provider) => (
              <button
                key={provider.id}
                type="button"
                onClick={() => loginWithSso(provider.id)}
                className="w-full rounded-xl border border-[#2a2520] bg-[#161412] py-2.5 text-[14px] font-medium text-[#e8e0d4] transition-colors hover:border-amber-500/30 hover:bg-[#1c1a17]"
              >
                {provider.label}
              </button>
            ))}
          </div>
        )}

        {(error || authError) && (
          <div className="rounded-xl border border-red-500/20 bg-red-500/[0.07] px-4 py-2.5 text-[13px] text-red-400">
            {formatAuthError(error || authError || "")}
          </div>
        )}

        {providerButtons.length > 0 && !needsSetup ? (
          <div>
            <button
              type="button"
              onClick={() => setShowManual(!showManual)}
              className="flex w-full items-center justify-center gap-2 text-[12px] text-zinc-500 transition-colors hover:text-zinc-400"
            >
              <span>Sign in with password</span>
              <svg
                className={`h-3 w-3 transition-transform ${showManual ? "rotate-180" : ""}`}
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
              >
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            </button>
            {showManual && (
              <form onSubmit={handleSubmit} className="mt-4 space-y-4">
                <div>
                  <label className="mb-1.5 block text-[12px] text-zinc-500">Username</label>
                  <input
                    value={username}
                    onChange={(e) => setUsername(e.target.value)}
                    className="w-full rounded-xl border border-[#2a2520] bg-[#1c1a17] px-4 py-2.5 text-[14px] text-[#e8e0d4] outline-none focus:border-amber-500/30"
                    placeholder="admin"
                  />
                </div>
                <div>
                  <label className="mb-1.5 block text-[12px] text-zinc-500">Password</label>
                  <input
                    type="password"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    className="w-full rounded-xl border border-[#2a2520] bg-[#1c1a17] px-4 py-2.5 text-[14px] text-[#e8e0d4] outline-none focus:border-amber-500/30"
                    placeholder="••••••"
                  />
                </div>
                <button
                  type="submit"
                  disabled={busy || !username.trim() || !password}
                  className="w-full rounded-xl bg-amber-500/20 py-2.5 text-[14px] font-medium text-amber-400 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/30 disabled:opacity-50"
                >
                  {busy ? "..." : "Sign In"}
                </button>
              </form>
            )}
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label className="mb-1.5 block text-[12px] text-zinc-500">Username</label>
              <input
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoFocus
                className="w-full rounded-xl border border-[#2a2520] bg-[#1c1a17] px-4 py-2.5 text-[14px] text-[#e8e0d4] outline-none focus:border-amber-500/30"
                placeholder="admin"
              />
            </div>
            {needsSetup && (
              <div>
                <label className="mb-1.5 block text-[12px] text-zinc-500">Display Name</label>
                <input
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  className="w-full rounded-xl border border-[#2a2520] bg-[#1c1a17] px-4 py-2.5 text-[14px] text-[#e8e0d4] outline-none focus:border-amber-500/30"
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
                className="w-full rounded-xl border border-[#2a2520] bg-[#1c1a17] px-4 py-2.5 text-[14px] text-[#e8e0d4] outline-none focus:border-amber-500/30"
                placeholder={needsSetup ? "Min 4 characters" : "••••••"}
              />
            </div>
            <button
              type="submit"
              disabled={busy || !username.trim() || !password}
              className="w-full rounded-xl bg-amber-500/20 py-2.5 text-[14px] font-medium text-amber-400 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/30 disabled:opacity-50"
            >
              {busy ? "..." : needsSetup ? "Create Account" : "Sign In"}
            </button>
          </form>
        )}
      </div>
    </div>
  );
}
