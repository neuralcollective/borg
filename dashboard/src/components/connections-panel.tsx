import { useQueryClient } from "@tanstack/react-query";
import { Github, MessageCircle, Plug } from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { useState } from "react";
import {
  connectDiscordBot,
  connectTelegramBot,
  disconnectDiscordBot,
  disconnectTelegramBot,
  type UserSettings,
  updateUserSettings,
  useUserSettings,
  useWhatsAppStatus,
} from "@/lib/api";
import { cn } from "@/lib/utils";

export function ConnectionsPanel() {
  return (
    <div className="flex h-full flex-col">
      <div className="shrink-0 space-y-3 p-5 pb-3">
        <div className="flex items-center gap-3">
          <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-amber-500/10 ring-1 ring-amber-500/20">
            <Plug className="h-6 w-6 text-amber-400" />
          </div>
          <div>
            <div className="text-[16px] font-semibold text-[#e8e0d4]">Connections</div>
            <div className="text-[13px] text-[#6b6459]">Connect external services to extend your workflow</div>
          </div>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-5">
        <div className="mx-auto grid max-w-3xl grid-cols-1 gap-4 md:grid-cols-2">
          <DiscordCard />
          <TelegramCard />
          <WhatsAppCard />
          <SlackCard />
          <GitHubCard />
          <GitLabCard />
          <CodebergCard />
        </div>
      </div>
    </div>
  );
}

// ── Discord ───────────────────────────────────────────────────────────────

function DiscordCard() {
  const queryClient = useQueryClient();
  const { data: userSettings } = useUserSettings();
  const [editing, setEditing] = useState(false);
  const [token, setToken] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  if (!userSettings) return null;

  const connected = userSettings.discord_bot_connected;
  const botUsername = userSettings.discord_bot_username;

  async function handleConnect() {
    setSaving(true);
    setError("");
    try {
      await connectDiscordBot(token);
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
      await disconnectDiscordBot();
      queryClient.invalidateQueries({ queryKey: ["user-settings"] });
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader
        icon={<DiscordIcon />}
        iconBg="bg-[#5865F2]/10 ring-[#5865F2]/20"
        title="Discord"
        subtitle="Chat with your agent from any Discord server or DM"
        status={connected ? "connected" : undefined}
        statusLabel={connected ? botUsername : undefined}
      />

      {connected && !editing ? (
        <div className="flex items-center gap-2 pt-1">
          <button
            onClick={() => setEditing(true)}
            className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#9c9486] transition-colors hover:bg-[#232019] hover:text-[#e8e0d4]"
          >
            Change Bot
          </button>
          <button
            onClick={handleDisconnect}
            disabled={saving}
            className="rounded-lg border border-red-500/20 bg-red-500/[0.06] px-3 py-1.5 text-[12px] text-red-400/80 transition-colors hover:bg-red-500/[0.12] hover:text-red-400"
          >
            Disconnect
          </button>
        </div>
      ) : (
        <div className="space-y-3 pt-1">
          <div className="rounded-xl border border-[#2a2520] bg-[#1c1a17]/60 px-4 py-3 text-[12px] text-[#9c9486] space-y-2">
            <p className="font-medium text-[#e8e0d4]">Setup</p>
            <ol className="list-decimal list-inside space-y-1.5 text-[12px]">
              <li>
                Go to the <span className="font-medium text-[#e8e0d4]">Discord Developer Portal</span>
              </li>
              <li>Create a new Application, then add a Bot</li>
              <li>
                Enable <span className="font-medium text-[#e8e0d4]">Message Content Intent</span> under Privileged
                Gateway Intents
              </li>
              <li>Copy the bot token and paste it below</li>
            </ol>
          </div>
          <div className="flex items-center gap-2">
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="Paste bot token"
              className="flex-1 rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[13px] text-[#e8e0d4] outline-none transition-colors focus:border-amber-500/30 placeholder:text-[#4a4540]"
              autoFocus
            />
            <button
              onClick={handleConnect}
              disabled={saving || !token.trim()}
              className={cn(
                "rounded-lg bg-amber-500/15 px-4 py-2 text-[12px] font-medium text-amber-300 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/20",
                (saving || !token.trim()) && "opacity-40 cursor-not-allowed",
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
                className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[12px] text-[#9c9486] transition-colors hover:text-[#e8e0d4]"
              >
                Cancel
              </button>
            )}
          </div>
          {error && <div className="text-[12px] text-red-400">{error}</div>}
        </div>
      )}
    </Card>
  );
}

// ── Telegram ──────────────────────────────────────────────────────────────

function TelegramCard() {
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
    <Card>
      <CardHeader
        icon={<TelegramIcon />}
        iconBg="bg-[#229ED9]/10 ring-[#229ED9]/20"
        title="Telegram"
        subtitle="Chat with your agent from any Telegram conversation"
        status={connected ? "connected" : undefined}
        statusLabel={connected ? `@${botUsername}` : undefined}
      />

      {connected && !editing ? (
        <div className="flex items-center gap-2 pt-1">
          <button
            onClick={() => setEditing(true)}
            className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#9c9486] transition-colors hover:bg-[#232019] hover:text-[#e8e0d4]"
          >
            Change Bot
          </button>
          <button
            onClick={handleDisconnect}
            disabled={saving}
            className="rounded-lg border border-red-500/20 bg-red-500/[0.06] px-3 py-1.5 text-[12px] text-red-400/80 transition-colors hover:bg-red-500/[0.12] hover:text-red-400"
          >
            Disconnect
          </button>
        </div>
      ) : (
        <div className="space-y-3 pt-1">
          <div className="rounded-xl border border-[#2a2520] bg-[#1c1a17]/60 px-4 py-3 text-[12px] text-[#9c9486] space-y-2">
            <p className="font-medium text-[#e8e0d4]">Setup</p>
            <ol className="list-decimal list-inside space-y-1.5 text-[12px]">
              <li>
                Open <span className="font-medium text-[#e8e0d4]">@BotFather</span> in Telegram
              </li>
              <li>
                Send <code className="rounded bg-[#2a2520] px-1.5 py-0.5 text-[11px] text-amber-300">/newbot</code> and
                follow the prompts
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
              className="flex-1 rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[13px] text-[#e8e0d4] outline-none transition-colors focus:border-amber-500/30 placeholder:text-[#4a4540]"
              autoFocus
            />
            <button
              onClick={handleConnect}
              disabled={saving || !token.trim()}
              className={cn(
                "rounded-lg bg-amber-500/15 px-4 py-2 text-[12px] font-medium text-amber-300 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/20",
                (saving || !token.trim()) && "opacity-40 cursor-not-allowed",
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
                className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[12px] text-[#9c9486] transition-colors hover:text-[#e8e0d4]"
              >
                Cancel
              </button>
            )}
          </div>
          {error && <div className="text-[12px] text-red-400">{error}</div>}
        </div>
      )}
    </Card>
  );
}

// ── WhatsApp ──────────────────────────────────────────────────────────────

function WhatsAppCard() {
  const { data: waStatus, isLoading } = useWhatsAppStatus();

  if (isLoading || !waStatus) return null;
  if (waStatus.disabled) return null;

  const jidLabel = waStatus.jid ? waStatus.jid.split("@")[0].split(":")[0] : undefined;

  return (
    <Card>
      <CardHeader
        icon={<WhatsAppIcon />}
        iconBg="bg-[#25D366]/10 ring-[#25D366]/20"
        title="WhatsApp"
        subtitle="Chat with your agent from any WhatsApp conversation"
        status={waStatus.connected ? "connected" : undefined}
        statusLabel={waStatus.connected ? jidLabel : undefined}
      />

      {waStatus.connected ? (
        <div className="rounded-xl border border-emerald-500/15 bg-emerald-500/[0.04] px-4 py-3 text-[12px] text-[#9c9486]">
          Connected and receiving messages
        </div>
      ) : waStatus.qr ? (
        <div className="space-y-3 pt-1">
          <div className="rounded-xl border border-[#2a2520] bg-[#1c1a17]/60 px-4 py-3 text-[12px] text-[#9c9486] space-y-2">
            <p className="font-medium text-[#e8e0d4]">Scan to connect</p>
            <p>
              Open WhatsApp on your phone, go to <span className="text-[#e8e0d4]">Linked Devices</span>, and scan this
              QR code.
            </p>
          </div>
          <div className="flex justify-center rounded-xl border border-[#2a2520] bg-white p-4">
            <QRCodeSVG value={waStatus.qr} size={200} />
          </div>
        </div>
      ) : (
        <div className="flex items-center gap-3 rounded-xl border border-dashed border-[#2a2520] px-4 py-4">
          <MessageCircle className="h-4 w-4 shrink-0 text-[#4a4540]" />
          <span className="text-[12px] text-[#6b6459]">Waiting for connection...</span>
        </div>
      )}
    </Card>
  );
}

// ── GitHub ─────────────────────────────────────────────────────────────────

function GitHubCard() {
  const { data: userSettings } = useUserSettings();
  if (!userSettings) return null;
  return (
    <PatCard
      icon={<Github className="h-4.5 w-4.5 text-[#e8e0d4]" />}
      iconBg="bg-[#e8e0d4]/8 ring-[#e8e0d4]/15"
      title="GitHub"
      subtitle="Personal access token for pushing branches, creating PRs, and cloning private repos"
      isSet={userSettings.github_token_set}
      placeholder="ghp_..."
      settingKey="github_token"
    />
  );
}

// ── GitLab ─────────────────────────────────────────────────────────────────

function GitLabIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-4.5 w-4.5" fill="currentColor">
      <path d="M22.65 14.39L12 22.13 1.35 14.39a.84.84 0 0 1-.3-.94l1.22-3.78 2.44-7.51A.42.42 0 0 1 4.82 2a.43.43 0 0 1 .58 0 .42.42 0 0 1 .11.18l2.44 7.49h8.1l2.44-7.51A.42.42 0 0 1 18.6 2a.43.43 0 0 1 .58 0 .42.42 0 0 1 .11.18l2.44 7.51L23 13.45a.84.84 0 0 1-.35.94z" />
    </svg>
  );
}

function GitLabCard() {
  const { data: userSettings } = useUserSettings();
  if (!userSettings) return null;
  return (
    <PatCard
      icon={<GitLabIcon />}
      iconBg="bg-[#FC6D26]/8 ring-[#FC6D26]/15"
      title="GitLab"
      subtitle="Personal access token for cloning private GitLab repos"
      isSet={userSettings.gitlab_token_set}
      placeholder="glpat-..."
      settingKey="gitlab_token"
    />
  );
}

// ── Codeberg ───────────────────────────────────────────────────────────────

function CodebergIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-4.5 w-4.5" fill="currentColor">
      <path d="M11.955.49A11.955 11.955 0 0 0 0 12.444a11.955 11.955 0 0 0 11.955 11.955 11.955 11.955 0 0 0 11.955-11.955A11.955 11.955 0 0 0 11.955.489zm0 1.64a10.315 10.315 0 0 1 10.315 10.315 10.315 10.315 0 0 1-10.315 10.315A10.315 10.315 0 0 1 1.64 12.445 10.315 10.315 0 0 1 11.955 2.13zM8.682 6.968v.002c-.43 0-.863.195-1.145.571L4.1 12.119a1.452 1.452 0 0 0 0 1.714l3.437 4.578c.564.753 1.727.753 2.291 0l.604-.804-2.833-3.774a.484.484 0 0 1 0-.572l2.833-3.772-.604-.805a1.452 1.452 0 0 0-1.146-.516zm6.636 0c-.43 0-.863.195-1.145.571l-.604.805 2.833 3.772a.484.484 0 0 1 0 .572l-2.833 3.774.604.804c.564.753 1.727.753 2.291 0l3.437-4.578a1.452 1.452 0 0 0 0-1.714l-3.437-4.578a1.452 1.452 0 0 0-1.146-.428z" />
    </svg>
  );
}

function CodebergCard() {
  const { data: userSettings } = useUserSettings();
  if (!userSettings) return null;
  return (
    <PatCard
      icon={<CodebergIcon />}
      iconBg="bg-[#2185D0]/8 ring-[#2185D0]/15"
      title="Codeberg"
      subtitle="Personal access token for cloning private Codeberg repos"
      isSet={userSettings.codeberg_token_set}
      placeholder="codeberg PAT..."
      settingKey="codeberg_token"
    />
  );
}

// ── Shared PAT card ────────────────────────────────────────────────────────

function PatCard({
  icon,
  iconBg,
  title,
  subtitle,
  isSet,
  placeholder,
  settingKey,
}: {
  icon: React.ReactNode;
  iconBg: string;
  title: string;
  subtitle: string;
  isSet: boolean;
  placeholder: string;
  settingKey: string;
}) {
  const { refetch } = useUserSettings();
  const [editing, setEditing] = useState(false);
  const [token, setToken] = useState("");
  const [saving, setSaving] = useState(false);

  async function handleSave() {
    setSaving(true);
    try {
      await updateUserSettings({ [settingKey]: token } as Partial<UserSettings>);
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
      await updateUserSettings({ [settingKey]: "" } as Partial<UserSettings>);
      await refetch();
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader
        icon={icon}
        iconBg={iconBg}
        title={title}
        subtitle={subtitle}
        status={isSet ? "connected" : undefined}
        statusLabel={isSet ? "Token configured" : undefined}
      />
      {isSet && !editing ? (
        <div className="flex items-center gap-2 pt-1">
          <button
            onClick={() => setEditing(true)}
            className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#9c9486] transition-colors hover:bg-[#232019] hover:text-[#e8e0d4]"
          >
            Update Token
          </button>
          <button
            onClick={handleClear}
            disabled={saving}
            className="rounded-lg border border-red-500/20 bg-red-500/[0.06] px-3 py-1.5 text-[12px] text-red-400/80 transition-colors hover:bg-red-500/[0.12] hover:text-red-400"
          >
            Remove
          </button>
        </div>
      ) : (
        <div className="space-y-2 pt-1">
          <div className="flex items-center gap-2">
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && handleSave()}
              placeholder={placeholder}
              autoFocus={editing}
              className="flex-1 rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[13px] text-[#e8e0d4] outline-none transition-colors focus:border-amber-500/30 placeholder:text-[#4a4540]"
            />
            <button
              onClick={handleSave}
              disabled={saving || !token.trim()}
              className={cn(
                "rounded-lg bg-amber-500/15 px-4 py-2 text-[12px] font-medium text-amber-300 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/20",
                (saving || !token.trim()) && "opacity-40 cursor-not-allowed",
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
                className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[12px] text-[#9c9486] transition-colors hover:text-[#e8e0d4]"
              >
                Cancel
              </button>
            )}
          </div>
        </div>
      )}
    </Card>
  );
}

// ── Slack ──────────────────────────────────────────────────────────────────

function SlackCard() {
  return (
    <Card>
      <CardHeader
        icon={<SlackIcon />}
        iconBg="bg-[#E01E5A]/8 ring-[#E01E5A]/15"
        title="Slack"
        subtitle="Chat with your agent from any Slack channel"
      />
      <div className="flex items-center gap-3 rounded-xl border border-dashed border-[#2a2520] px-4 py-4">
        <MessageCircle className="h-4 w-4 shrink-0 text-[#4a4540]" />
        <span className="text-[12px] text-[#6b6459]">Coming soon</span>
      </div>
    </Card>
  );
}

// ── Shared UI ─────────────────────────────────────────────────────────────

function Card({ children }: { children: React.ReactNode }) {
  return <div className="rounded-2xl border border-[#2a2520] bg-[#151412] p-5 space-y-3">{children}</div>;
}

function CardHeader({
  icon,
  iconBg,
  title,
  subtitle,
  status,
  statusLabel,
}: {
  icon: React.ReactNode;
  iconBg: string;
  title: string;
  subtitle: string;
  status?: "connected";
  statusLabel?: string;
}) {
  return (
    <div className="flex items-start gap-3.5">
      <div className={cn("flex h-10 w-10 shrink-0 items-center justify-center rounded-xl ring-1", iconBg)}>{icon}</div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2.5">
          <span className="text-[14px] font-semibold text-[#e8e0d4]">{title}</span>
          {status && (
            <span className="inline-flex items-center gap-1.5 rounded-full border border-emerald-500/25 bg-emerald-500/[0.08] px-2.5 py-0.5 text-[11px] font-medium text-emerald-400">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
              {statusLabel ?? "Connected"}
            </span>
          )}
        </div>
        <p className="mt-0.5 text-[12px] leading-relaxed text-[#6b6459]">{subtitle}</p>
      </div>
    </div>
  );
}

function TelegramIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className="h-4.5 w-4.5 text-[#229ED9]">
      <path d="M11.944 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0a12 12 0 0 0-.056 0zm4.962 7.224c.1-.002.321.023.465.14a.506.506 0 0 1 .171.325c.016.093.036.306.02.472-.18 1.898-.962 6.502-1.36 8.627-.168.9-.499 1.201-.82 1.23-.696.065-1.225-.46-1.9-.902-1.056-.693-1.653-1.124-2.678-1.8-1.185-.78-.417-1.21.258-1.91.177-.184 3.247-2.977 3.307-3.23.007-.032.014-.15-.056-.212s-.174-.041-.249-.024c-.106.024-1.793 1.14-5.061 3.345-.48.33-.913.49-1.302.48-.428-.008-1.252-.241-1.865-.44-.752-.245-1.349-.374-1.297-.789.027-.216.325-.437.893-.663 3.498-1.524 5.83-2.529 6.998-3.014 3.332-1.386 4.025-1.627 4.476-1.635z" />
    </svg>
  );
}

function DiscordIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className="h-4.5 w-4.5 text-[#5865F2]">
      <path d="M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189z" />
    </svg>
  );
}

function WhatsAppIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className="h-4.5 w-4.5 text-[#25D366]">
      <path d="M17.472 14.382c-.297-.149-1.758-.867-2.03-.967-.273-.099-.471-.148-.67.15-.197.297-.767.966-.94 1.164-.173.199-.347.223-.644.075-.297-.15-1.255-.463-2.39-1.475-.883-.788-1.48-1.761-1.653-2.059-.173-.297-.018-.458.13-.606.134-.133.298-.347.446-.52.149-.174.198-.298.298-.497.099-.198.05-.371-.025-.52-.075-.149-.669-1.612-.916-2.207-.242-.579-.487-.5-.669-.51-.173-.008-.371-.01-.57-.01-.198 0-.52.074-.792.372-.272.297-1.04 1.016-1.04 2.479 0 1.462 1.065 2.875 1.213 3.074.149.198 2.096 3.2 5.077 4.487.709.306 1.262.489 1.694.625.712.227 1.36.195 1.871.118.571-.085 1.758-.719 2.006-1.413.248-.694.248-1.289.173-1.413-.074-.124-.272-.198-.57-.347m-5.421 7.403h-.004a9.87 9.87 0 01-5.031-1.378l-.361-.214-3.741.982.998-3.648-.235-.374a9.86 9.86 0 01-1.51-5.26c.001-5.45 4.436-9.884 9.888-9.884 2.64 0 5.122 1.03 6.988 2.898a9.825 9.825 0 012.893 6.994c-.003 5.45-4.437 9.884-9.885 9.884m8.413-18.297A11.815 11.815 0 0012.05 0C5.495 0 .16 5.335.157 11.892c0 2.096.547 4.142 1.588 5.945L.057 24l6.305-1.654a11.882 11.882 0 005.683 1.448h.005c6.554 0 11.89-5.335 11.893-11.893a11.821 11.821 0 00-3.48-8.413z" />
    </svg>
  );
}

function SlackIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className="h-4.5 w-4.5">
      <path
        d="M5.042 15.165a2.528 2.528 0 0 1-2.52 2.523A2.528 2.528 0 0 1 0 15.165a2.527 2.527 0 0 1 2.522-2.52h2.52v2.52zm1.271 0a2.527 2.527 0 0 1 2.521-2.52 2.527 2.527 0 0 1 2.521 2.52v6.313A2.528 2.528 0 0 1 8.834 24a2.528 2.528 0 0 1-2.521-2.522v-6.313zM8.834 5.042a2.528 2.528 0 0 1-2.521-2.52A2.528 2.528 0 0 1 8.834 0a2.528 2.528 0 0 1 2.521 2.522v2.52H8.834zm0 1.271a2.528 2.528 0 0 1 2.521 2.521 2.528 2.528 0 0 1-2.521 2.521H2.522A2.528 2.528 0 0 1 0 8.834a2.528 2.528 0 0 1 2.522-2.521h6.312zm10.122 2.521a2.528 2.528 0 0 1 2.522-2.521A2.528 2.528 0 0 1 24 8.834a2.528 2.528 0 0 1-2.522 2.521h-2.522V8.834zm-1.268 0a2.528 2.528 0 0 1-2.523 2.521 2.527 2.527 0 0 1-2.52-2.521V2.522A2.527 2.527 0 0 1 15.165 0a2.528 2.528 0 0 1 2.523 2.522v6.312zm-2.523 10.122a2.528 2.528 0 0 1 2.523 2.522A2.528 2.528 0 0 1 15.165 24a2.527 2.527 0 0 1-2.52-2.522v-2.522h2.52zm0-1.268a2.527 2.527 0 0 1-2.52-2.523 2.526 2.526 0 0 1 2.52-2.52h6.313A2.527 2.527 0 0 1 24 15.165a2.528 2.528 0 0 1-2.522 2.523h-6.313z"
        fill="#E01E5A"
      />
    </svg>
  );
}
