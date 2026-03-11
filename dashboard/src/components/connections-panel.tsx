import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import {
  connectTelegramBot,
  disconnectTelegramBot,
  updateUserSettings,
  useUserSettings,
  type UserSettings,
} from "@/lib/api";
import { cn } from "@/lib/utils";

export function ConnectionsPanel() {
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-2xl space-y-8 px-6 py-8">
        <TelegramSection />
        <SlackSection />
        <GitHubSection />
      </div>
    </div>
  );
}

// ── Telegram ──────────────────────────────────────────────────────────────

function TelegramSection() {
  const queryClient = useQueryClient();
  const { data: userSettings } = useUserSettings();
  const [editing, setEditing] = useState(false);
  const [token, setToken] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  if (!userSettings) return null;

  const connected = userSettings.telegram_bot_connected;
  const botUsername = userSettings.telegram_bot_username;

  async function handleConnect() {
    setSaving(true);
    setError("");
    try {
      await connectTelegramBot(token);
      setToken("");
      setEditing(false);
      queryClient.invalidateQueries({ queryKey: ["user-settings"] });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to connect");
    } finally {
      setSaving(false);
    }
  }

  async function handleDisconnect() {
    setSaving(true);
    try {
      await disconnectTelegramBot();
      queryClient.invalidateQueries({ queryKey: ["user-settings"] });
    } finally {
      setSaving(false);
    }
  }

  return (
    <Section
      title="Telegram"
      description="Connect your own Telegram bot so you can chat with Borg from any Telegram conversation."
      status={connected ? `@${botUsername}` : undefined}
    >
      {connected && !editing ? (
        <div className="flex items-center justify-between rounded-lg border border-[#2a2520] bg-[#1c1a17]/50 px-4 py-3">
          <div className="flex items-center gap-3">
            <div className="flex h-8 w-8 items-center justify-center rounded-full bg-[#229ED9]/15 text-[13px]">
              <TelegramIcon />
            </div>
            <div>
              <div className="text-[12px] font-medium text-[#e8e0d4]">@{botUsername}</div>
              <div className="text-[11px] text-emerald-400">Connected</div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setEditing(true)}
              className="rounded-md border border-[#2a2520] bg-[#1c1a17] px-2.5 py-1.5 text-[12px] text-[#9c9486] hover:text-[#e8e0d4] transition-colors"
            >
              Change
            </button>
            <button
              onClick={handleDisconnect}
              disabled={saving}
              className="rounded-md border border-[#2a2520] bg-[#1c1a17] px-2.5 py-1.5 text-[12px] text-red-400/70 hover:text-red-400 transition-colors"
            >
              Disconnect
            </button>
          </div>
        </div>
      ) : (
        <div className="space-y-3">
          <div className="rounded-lg border border-[#2a2520] bg-[#1c1a17]/50 px-4 py-3 text-[12px] text-[#9c9486] space-y-2">
            <p>To connect Telegram:</p>
            <ol className="list-decimal list-inside space-y-1 text-[11px]">
              <li>
                Open{" "}
                <span className="text-[#e8e0d4]">@BotFather</span> in Telegram
              </li>
              <li>
                Send <span className="font-mono text-[#e8e0d4]">/newbot</span> and follow the prompts to create a bot
              </li>
              <li>Copy the bot token and paste it below</li>
            </ol>
          </div>
          <div className="flex items-center gap-2">
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="Paste bot token from @BotFather"
              className="flex-1 rounded-md border border-[#2a2520] bg-[#1c1a17] px-2.5 py-1.5 text-[12px] text-[#e8e0d4] outline-none focus:border-amber-500/40 placeholder:text-[#4a4540]"
              autoFocus
            />
            <button
              onClick={handleConnect}
              disabled={saving || !token.trim()}
              className={cn(
                "rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-[12px] font-medium text-amber-400 transition-colors hover:bg-amber-500/20",
                (saving || !token.trim()) && "opacity-50 cursor-not-allowed",
              )}
            >
              {saving ? "Verifying..." : "Connect"}
            </button>
            {connected && (
              <button
                onClick={() => {
                  setEditing(false);
                  setToken("");
                  setError("");
                }}
                className="rounded-md border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#9c9486] hover:text-[#e8e0d4] transition-colors"
              >
                Cancel
              </button>
            )}
          </div>
          {error && <div className="text-[11px] text-red-400">{error}</div>}
        </div>
      )}
    </Section>
  );
}

// ── Slack ──────────────────────────────────────────────────────────────────

function SlackSection() {
  return (
    <Section
      title="Slack"
      description="Connect a Slack workspace to chat with Borg from any channel."
    >
      <div className="rounded-lg border border-[#2a2520] bg-[#1c1a17]/50 px-4 py-6 text-center">
        <div className="mb-2 text-[13px] text-[#6b6459]">Coming soon</div>
        <div className="text-[11px] text-[#4a4540]">
          Slack integration is under development.
        </div>
      </div>
    </Section>
  );
}

// ── GitHub ─────────────────────────────────────────────────────────────────

function GitHubSection() {
  const queryClient = useQueryClient();
  const { data: userSettings, refetch } = useUserSettings();
  const [editing, setEditing] = useState(false);
  const [token, setToken] = useState("");
  const [saving, setSaving] = useState(false);

  if (!userSettings) return null;

  const isSet = userSettings.github_token_set;

  async function handleSave() {
    setSaving(true);
    try {
      await updateUserSettings({ github_token: token } as Partial<UserSettings>);
      setToken("");
      setEditing(false);
      await refetch();
    } finally {
      setSaving(false);
    }
  }

  async function handleClear() {
    setSaving(true);
    try {
      await updateUserSettings({ github_token: "" } as Partial<UserSettings>);
      await refetch();
    } finally {
      setSaving(false);
    }
  }

  return (
    <Section
      title="GitHub"
      description="Personal access token for pushing branches and creating PRs under your account. Leave blank to use the system default."
      status={isSet ? "Configured" : undefined}
    >
      {isSet && !editing ? (
        <div className="flex items-center justify-between rounded-lg border border-[#2a2520] bg-[#1c1a17]/50 px-4 py-3">
          <div className="flex items-center gap-3">
            <div className="flex h-8 w-8 items-center justify-center rounded-full bg-[#e8e0d4]/10 text-[13px]">
              <GitHubIcon />
            </div>
            <div>
              <div className="text-[12px] font-medium text-[#e8e0d4]">Personal Access Token</div>
              <div className="text-[11px] text-emerald-400">Configured</div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setEditing(true)}
              className="rounded-md border border-[#2a2520] bg-[#1c1a17] px-2.5 py-1.5 text-[12px] text-[#9c9486] hover:text-[#e8e0d4] transition-colors"
            >
              Update
            </button>
            <button
              onClick={handleClear}
              disabled={saving}
              className="rounded-md border border-[#2a2520] bg-[#1c1a17] px-2.5 py-1.5 text-[12px] text-red-400/70 hover:text-red-400 transition-colors"
            >
              Remove
            </button>
          </div>
        </div>
      ) : (
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="ghp_..."
              className="flex-1 rounded-md border border-[#2a2520] bg-[#1c1a17] px-2.5 py-1.5 text-[12px] text-[#e8e0d4] outline-none focus:border-amber-500/40 placeholder:text-[#4a4540]"
              autoFocus
            />
            <button
              onClick={handleSave}
              disabled={saving || !token.trim()}
              className={cn(
                "rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-[12px] font-medium text-amber-400 transition-colors hover:bg-amber-500/20",
                (saving || !token.trim()) && "opacity-50 cursor-not-allowed",
              )}
            >
              Save
            </button>
            {isSet && (
              <button
                onClick={() => {
                  setEditing(false);
                  setToken("");
                }}
                className="rounded-md border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#9c9486] hover:text-[#e8e0d4] transition-colors"
              >
                Cancel
              </button>
            )}
          </div>
        </div>
      )}
    </Section>
  );
}

// ── Shared UI ─────────────────────────────────────────────────────────────

function Section({
  title,
  description,
  status,
  children,
}: {
  title: string;
  description?: string;
  status?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-3">
        <h3 className="text-[13px] font-semibold text-[#e8e0d4]">{title}</h3>
        {status && (
          <span className="rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[10px] text-emerald-400">
            {status}
          </span>
        )}
      </div>
      {description && <p className="text-[11px] text-[#6b6459] -mt-1">{description}</p>}
      {children}
    </div>
  );
}

function TelegramIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4 text-[#229ED9]">
      <path d="M11.944 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0a12 12 0 0 0-.056 0zm4.962 7.224c.1-.002.321.023.465.14a.506.506 0 0 1 .171.325c.016.093.036.306.02.472-.18 1.898-.962 6.502-1.36 8.627-.168.9-.499 1.201-.82 1.23-.696.065-1.225-.46-1.9-.902-1.056-.693-1.653-1.124-2.678-1.8-1.185-.78-.417-1.21.258-1.91.177-.184 3.247-2.977 3.307-3.23.007-.032.014-.15-.056-.212s-.174-.041-.249-.024c-.106.024-1.793 1.14-5.061 3.345-.48.33-.913.49-1.302.48-.428-.008-1.252-.241-1.865-.44-.752-.245-1.349-.374-1.297-.789.027-.216.325-.437.893-.663 3.498-1.524 5.83-2.529 6.998-3.014 3.332-1.386 4.025-1.627 4.476-1.635z" />
    </svg>
  );
}

function GitHubIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4 text-[#e8e0d4]">
      <path d="M12 0C5.374 0 0 5.373 0 12c0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23A11.509 11.509 0 0 1 12 5.803c1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576C20.566 21.797 24 17.3 24 12c0-6.627-5.373-12-12-12z" />
    </svg>
  );
}
