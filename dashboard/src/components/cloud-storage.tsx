import { Folder, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { CloudBrowseItem, CloudConnection, Settings } from "@/lib/api";
import {
  browseProjectCloudFiles,
  deleteProjectCloudConnection,
  importProjectCloudFiles,
  useProjectCloudConnections,
} from "@/lib/api";
import { cn } from "@/lib/utils";
import { formatFileSize } from "./file-list-shared";

const CLOUD_PROVIDERS = [
  { id: "dropbox", label: "Dropbox", clientIdKey: "dropbox_client_id", clientSecretKey: "dropbox_client_secret" },
  {
    id: "google_drive",
    label: "Google Drive",
    clientIdKey: "google_client_id",
    clientSecretKey: "google_client_secret",
  },
  { id: "onedrive", label: "OneDrive", clientIdKey: "ms_client_id", clientSecretKey: "ms_client_secret" },
] as const;

const MAX_CLOUD_IMPORT_SELECTION = 1000;

function cloudProviderLabel(provider: string): string {
  return CLOUD_PROVIDERS.find((p) => p.id === provider)?.label ?? provider;
}

function DropboxIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden>
      <path
        fill="#0D63D6"
        d="m6.1 3.2-4.7 3 4.7 3 4.7-3-4.7-3Zm11.8 0-4.7 3 4.7 3 4.7-3-4.7-3ZM6.1 10.7l-4.7 3 4.7 3 4.7-3-4.7-3Zm11.8 0-4.7 3 4.7 3 4.7-3-4.7-3ZM12 14.9l-4.7 3 4.7 3 4.7-3-4.7-3Z"
      />
    </svg>
  );
}

function GoogleDriveIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden>
      <path fill="#0F9D58" d="M6.5 20.3h11l-2.7-4.7h-11l2.7 4.7Z" />
      <path fill="#FFC107" d="m12 3.7 5.5 9.5h5.4L17.4 3.7H12Z" />
      <path fill="#4285F4" d="M1.1 13.2h5.4L12 3.7H6.6L1.1 13.2Z" />
    </svg>
  );
}

function OneDriveIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden>
      <path
        fill="#0078D4"
        d="M10.2 9a5.4 5.4 0 0 1 10.2 2.4h.2a3.4 3.4 0 1 1 0 6.8H6.5a4.5 4.5 0 0 1-.8-8.9A5.7 5.7 0 0 1 10.2 9Z"
      />
    </svg>
  );
}

function CloudProviderIcon({ provider }: { provider: string }) {
  if (provider === "dropbox") return <DropboxIcon />;
  if (provider === "google_drive") return <GoogleDriveIcon />;
  return <OneDriveIcon />;
}

interface CloudStoragePanelProps {
  projectId: number | null;
  settings: Settings | null;
  onImported: () => void;
}

export function CloudStoragePanel({ projectId, settings, onImported }: CloudStoragePanelProps) {
  const {
    data: cloudConnections = [],
    refetch: refetchCloudConnections,
    isLoading: cloudConnectionsLoading,
  } = useProjectCloudConnections(projectId);

  const [cloudMessage, setCloudMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);
  const [cloudModalOpen, setCloudModalOpen] = useState(false);
  const [cloudModalConn, setCloudModalConn] = useState<CloudConnection | null>(null);
  const [cloudItems, setCloudItems] = useState<CloudBrowseItem[]>([]);
  const [cloudLoading, setCloudLoading] = useState(false);
  const [cloudLoadError, setCloudLoadError] = useState<string | null>(null);
  const [cloudCursor, setCloudCursor] = useState<string | null>(null);
  const [cloudHasMore, setCloudHasMore] = useState(false);
  const [cloudSelected, setCloudSelected] = useState<Record<string, CloudBrowseItem>>({});
  const [cloudImporting, setCloudImporting] = useState(false);
  const [cloudBreadcrumbs, setCloudBreadcrumbs] = useState<Array<{ id?: string; name: string }>>([{ name: "Root" }]);

  const publicUrl = settings?.public_url?.trim() || "";
  const publicUrlValid = useMemo(() => {
    if (!publicUrl) return false;
    try {
      const parsed = new URL(publicUrl);
      return parsed.protocol === "http:" || parsed.protocol === "https:";
    } catch {
      return false;
    }
  }, [publicUrl]);

  const maxCloudImportSelection = Math.max(1, settings?.cloud_import_max_batch_files ?? MAX_CLOUD_IMPORT_SELECTION);
  const currentCloudFolderId = cloudBreadcrumbs[cloudBreadcrumbs.length - 1]?.id;

  // OAuth callback URL hash parsing
  useEffect(() => {
    const hash = window.location.hash || "";
    const queryIdx = hash.indexOf("?");
    if (queryIdx < 0) return;

    const params = new URLSearchParams(hash.slice(queryIdx + 1));
    const connected = params.get("cloud_connected");
    const error = params.get("cloud_error");
    const provider = params.get("provider");
    if (!connected && !error) return;

    if (connected) {
      setCloudMessage({ type: "success", text: `${cloudProviderLabel(connected)} connected.` });
      refetchCloudConnections();
    } else if (error) {
      const prefix = provider ? `${cloudProviderLabel(provider)}: ` : "";
      if (error === "access_denied") {
        setCloudMessage({ type: "error", text: `${prefix}authorization was denied.` });
      } else if (error === "token_exchange") {
        setCloudMessage({
          type: "error",
          text: `${prefix}token exchange failed. Check client ID/secret and callback URL.`,
        });
      } else if (error === "missing_public_url") {
        setCloudMessage({
          type: "error",
          text: "Set a valid Public URL in Settings before connecting cloud providers.",
        });
      } else if (error === "missing_credentials") {
        setCloudMessage({ type: "error", text: `${prefix}credentials are missing in Settings > Cloud Storage.` });
      } else {
        setCloudMessage({ type: "error", text: `${prefix}connection failed (${error}).` });
      }
    }

    const cleanHash = hash.slice(0, queryIdx) || "#/projects";
    window.history.replaceState(null, "", `${window.location.pathname}${window.location.search}${cleanHash}`);
  }, [refetchCloudConnections]);

  // Reset cloud state when projectId changes
  useEffect(() => {
    if (!projectId) {
      setCloudModalOpen(false);
      setCloudModalConn(null);
      setCloudItems([]);
      setCloudSelected({});
      setCloudBreadcrumbs([{ name: "Root" }]);
    }
  }, [projectId]);

  function hasCloudCredentials(provider: (typeof CLOUD_PROVIDERS)[number]) {
    if (!settings) return false;
    const id = settings[provider.clientIdKey] ?? "";
    const secret = settings[provider.clientSecretKey] ?? "";
    return id.trim().length > 0 && secret.trim().length > 0;
  }

  async function loadCloudFolder(
    connection: CloudConnection,
    folderId?: string,
    opts?: { append?: boolean; cursor?: string },
  ) {
    if (!projectId) return;
    setCloudLoading(true);
    setCloudLoadError(null);
    try {
      const data = await browseProjectCloudFiles(projectId, connection.id, {
        folder_id: folderId,
        cursor: opts?.cursor,
      });
      setCloudItems((prev) => (opts?.append ? [...prev, ...(data.items || [])] : data.items || []));
      const nextCursor = data.cursor ?? data.next_page_token ?? null;
      setCloudCursor(nextCursor);
      setCloudHasMore(Boolean(data.has_more || data.next_page_token));
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Failed to browse cloud files";
      setCloudLoadError(msg);
    } finally {
      setCloudLoading(false);
    }
  }

  function connectCloudProvider(provider: (typeof CLOUD_PROVIDERS)[number]["id"]) {
    if (!projectId) return;
    if (!publicUrlValid) {
      setCloudMessage({ type: "error", text: "Set a valid Public URL in Settings before connecting cloud providers." });
      return;
    }
    window.location.href = `/api/cloud/${provider}/auth?project_id=${projectId}`;
  }

  async function openCloudBrowser(connection: CloudConnection) {
    setCloudModalConn(connection);
    setCloudModalOpen(true);
    setCloudSelected({});
    setCloudBreadcrumbs([{ name: "Root" }]);
    setCloudCursor(null);
    setCloudHasMore(false);
    await loadCloudFolder(connection);
  }

  async function disconnectCloudConnection(connection: CloudConnection) {
    if (!projectId) return;
    if (
      !confirm(
        `Disconnect ${cloudProviderLabel(connection.provider)} account ${connection.account_email || connection.id}?`,
      )
    )
      return;
    try {
      await deleteProjectCloudConnection(projectId, connection.id);
      setCloudMessage({ type: "success", text: `${cloudProviderLabel(connection.provider)} disconnected.` });
      await refetchCloudConnections();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "disconnect failed";
      setCloudMessage({ type: "error", text: `Failed to disconnect (${msg}).` });
    }
  }

  async function importSelectedCloudFiles() {
    if (!projectId || !cloudModalConn || cloudImporting) return;
    const filesToImport = Object.values(cloudSelected)
      .filter((item) => item.type === "file")
      .map((item) => ({ id: item.id, name: item.name, size: item.size }));
    if (filesToImport.length === 0) return;
    if (filesToImport.length > maxCloudImportSelection) {
      setCloudLoadError(`Please select at most ${maxCloudImportSelection} files per import.`);
      return;
    }

    setCloudImporting(true);
    try {
      await importProjectCloudFiles(projectId, cloudModalConn.id, filesToImport);
      setCloudMessage({ type: "success", text: `Imported ${filesToImport.length} file(s).` });
      setCloudModalOpen(false);
      setCloudSelected({});
      onImported();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "import failed";
      setCloudLoadError(`Import failed (${msg}).`);
    } finally {
      setCloudImporting(false);
    }
  }

  return (
    <>
      {/* Cloud Storage panel */}
      <div className="rounded-xl border border-[#2a2520] bg-[#151412] p-4">
        <div className="mb-3 text-[12px] font-semibold text-[#e8e0d4]">Cloud Storage</div>
        {!publicUrlValid && (
          <div className="mb-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-300">
            Configure a valid Public URL in Settings before connecting cloud accounts.
          </div>
        )}
        {cloudMessage && (
          <div
            className={cn(
              "mb-3 flex items-start justify-between gap-2 rounded-lg border px-3 py-2 text-[11px]",
              cloudMessage.type === "success"
                ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-400"
                : "border-red-500/30 bg-red-500/10 text-red-400",
            )}
          >
            <span>{cloudMessage.text}</span>
            <button onClick={() => setCloudMessage(null)} className="shrink-0 text-[#6b6459] hover:text-[#e8e0d4]">
              <X className="h-3 w-3" />
            </button>
          </div>
        )}
        <div className="mb-3 flex flex-wrap gap-1.5">
          {CLOUD_PROVIDERS.map((provider) => {
            const configured = hasCloudCredentials(provider);
            return (
              <button
                key={provider.id}
                onClick={() => connectCloudProvider(provider.id)}
                disabled={!configured || !projectId || !publicUrlValid}
                title={
                  !publicUrlValid
                    ? "Set a valid Public URL in Settings > Cloud Storage"
                    : configured
                      ? `Connect ${provider.label}`
                      : `Configure ${provider.label} credentials in Settings > Cloud Storage`
                }
                className="inline-flex items-center gap-1.5 rounded-lg border border-[#2a2520] px-3 py-1.5 text-[12px] text-[#e8e0d4] transition-colors hover:bg-[#232019] disabled:cursor-not-allowed disabled:opacity-40"
              >
                <CloudProviderIcon provider={provider.id} />
                {provider.label}
              </button>
            );
          })}
        </div>
        <div className="space-y-1.5 max-h-36 overflow-y-auto">
          {cloudConnections.map((conn) => (
            <div
              key={conn.id}
              className="flex items-center justify-between rounded-lg border border-[#2a2520] px-3 py-2 text-[12px]"
            >
              <div className="min-w-0 flex items-center gap-1.5 text-[#e8e0d4]">
                <CloudProviderIcon provider={conn.provider} />
                <span className="truncate">{conn.account_email || cloudProviderLabel(conn.provider)}</span>
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                <button
                  onClick={() => openCloudBrowser(conn)}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-[#2a2520] px-2.5 py-1 text-[12px] text-[#e8e0d4] transition-colors hover:bg-[#232019]"
                >
                  <Folder className="h-3 w-3" />
                  Browse
                </button>
                <button
                  onClick={() => disconnectCloudConnection(conn)}
                  className="rounded-lg p-1.5 text-[#6b6459] transition-colors hover:bg-red-500/10 hover:text-red-400"
                  title="Disconnect"
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            </div>
          ))}
          {!cloudConnectionsLoading && cloudConnections.length === 0 && (
            <div className="text-[12px] text-[#6b6459]">No connected cloud accounts.</div>
          )}
        </div>
      </div>

      {/* Cloud browser modal */}
      {cloudModalOpen && cloudModalConn && projectId && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
          onClick={() => setCloudModalOpen(false)}
        >
          <div
            className="mx-4 flex max-h-[82vh] w-full max-w-4xl flex-col rounded-xl border border-white/10 bg-zinc-900 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between border-b border-white/10 px-5 py-4">
              <div className="min-w-0">
                <div className="text-[15px] font-semibold text-zinc-100">
                  {cloudProviderLabel(cloudModalConn.provider)} - {cloudModalConn.account_email || "Account"}
                </div>
                <div className="mt-1.5 flex items-center gap-1 overflow-x-auto text-[12px] text-zinc-400">
                  {cloudBreadcrumbs.map((crumb, idx) => (
                    <button
                      key={`${crumb.id ?? "root"}-${idx}`}
                      onClick={async () => {
                        const next = cloudBreadcrumbs.slice(0, idx + 1);
                        setCloudBreadcrumbs(next);
                        setCloudSelected({});
                        setCloudCursor(null);
                        setCloudHasMore(false);
                        await loadCloudFolder(cloudModalConn, next[next.length - 1]?.id);
                      }}
                      className="shrink-0 hover:text-zinc-300"
                    >
                      {idx > 0 ? "/" : ""}
                      {crumb.name}
                    </button>
                  ))}
                </div>
              </div>
              <button onClick={() => setCloudModalOpen(false)} className="text-zinc-500 hover:text-zinc-300">
                x
              </button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto p-4">
              {cloudLoadError && (
                <div className="mb-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-[12px] text-red-400">
                  {cloudLoadError}
                </div>
              )}
              <div className="overflow-hidden rounded-xl border border-white/[0.08]">
                {cloudItems.map((item) => {
                  const selected = Boolean(cloudSelected[item.id]);
                  return (
                    <div
                      key={item.id}
                      className="flex items-center justify-between border-b border-white/[0.07] px-3 py-2.5 text-[13px] last:border-b-0"
                    >
                      <label className="flex min-w-0 flex-1 items-center gap-2 text-zinc-300">
                        {item.type === "file" ? (
                          <input
                            type="checkbox"
                            checked={selected}
                            onChange={(e) => {
                              setCloudSelected((prev) => {
                                const next = { ...prev };
                                if (e.target.checked) next[item.id] = item;
                                else delete next[item.id];
                                return next;
                              });
                            }}
                          />
                        ) : (
                          <span className="inline-block w-4" />
                        )}
                        <button
                          disabled={item.type !== "folder"}
                          onClick={async () => {
                            if (item.type !== "folder") return;
                            setCloudBreadcrumbs((prev) => [...prev, { id: item.id, name: item.name }]);
                            setCloudSelected({});
                            setCloudCursor(null);
                            setCloudHasMore(false);
                            await loadCloudFolder(cloudModalConn, item.id);
                          }}
                          className={cn(
                            "truncate text-left",
                            item.type === "folder" ? "text-blue-400 hover:text-blue-300" : "text-zinc-300",
                          )}
                        >
                          {item.type === "folder" ? "[DIR] " : "[FILE] "}
                          {item.name}
                        </button>
                      </label>
                      <div className="ml-2 shrink-0 text-[12px] text-zinc-500">
                        {item.type === "file" ? formatFileSize(item.size || 0) : "folder"}
                      </div>
                    </div>
                  );
                })}
                {!cloudLoading && cloudItems.length === 0 && (
                  <div className="px-4 py-6 text-[13px] text-zinc-500 text-center">This folder is empty.</div>
                )}
              </div>
              {cloudLoading && <div className="mt-3 text-[12px] text-zinc-500">Loading...</div>}
              {!cloudLoading && cloudHasMore && cloudCursor && (
                <button
                  onClick={() =>
                    loadCloudFolder(cloudModalConn, currentCloudFolderId, { append: true, cursor: cloudCursor })
                  }
                  className="mt-3 rounded-lg border border-white/[0.08] px-3 py-1.5 text-[12px] text-zinc-300 hover:bg-white/[0.06] transition-colors"
                >
                  Load more
                </button>
              )}
            </div>
            <div className="flex items-center justify-between border-t border-white/10 px-5 py-4">
              <div className="text-[12px] text-zinc-400">
                Selected: {Object.values(cloudSelected).filter((i) => i.type === "file").length} file(s)
              </div>
              <button
                onClick={importSelectedCloudFiles}
                disabled={cloudImporting || Object.values(cloudSelected).every((i) => i.type !== "file")}
                className="rounded-lg bg-blue-500/20 px-4 py-2 text-[13px] font-medium text-blue-300 hover:bg-blue-500/30 transition-colors disabled:cursor-not-allowed disabled:text-zinc-600"
              >
                {cloudImporting ? "Importing..." : "Import Selected"}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
