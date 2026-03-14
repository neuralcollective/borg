import { GitBranch, Plus, RotateCw, Trash2 } from "lucide-react";
import { useState } from "react";
import {
  addKnowledgeRepo,
  deleteKnowledgeRepo,
  retryKnowledgeRepo,
  useKnowledgeRepos,
} from "@/lib/api";
import type { KnowledgeRepo } from "@/lib/types";

function repoErrorHint(errorMsg: string): string | null {
  if (errorMsg.includes("terminal prompts disabled") || errorMsg.includes("could not read Username")) {
    return "Add your GitHub token in Connections to clone private repos";
  }
  if (errorMsg.includes("not found") || errorMsg.includes("404")) {
    return "Repository not found — check the URL";
  }
  return null;
}

export function GitReposPanel({
  isOrg,
  accentBg,
  accentText,
}: {
  isOrg: boolean;
  accentBg: string;
  accentText: string;
}) {
  const { data: repoData, refetch: refetchRepos } = useKnowledgeRepos(isOrg);
  const repos = repoData?.repos ?? [];
  const [addRepoOpen, setAddRepoOpen] = useState(false);
  const [addRepoUrl, setAddRepoUrl] = useState("");
  const [addRepoName, setAddRepoName] = useState("");
  const [addRepoLoading, setAddRepoLoading] = useState(false);
  const [addRepoError, setAddRepoError] = useState<string | null>(null);

  async function handleAddRepo() {
    if (!addRepoUrl.trim()) return;
    setAddRepoLoading(true);
    setAddRepoError(null);
    try {
      await addKnowledgeRepo(isOrg, addRepoUrl.trim(), addRepoName.trim() || undefined);
      setAddRepoUrl("");
      setAddRepoName("");
      setAddRepoOpen(false);
      refetchRepos();
    } catch (err) {
      setAddRepoError(err instanceof Error ? err.message : "Failed to add repo");
    } finally {
      setAddRepoLoading(false);
    }
  }

  async function handleDeleteRepo(repo: KnowledgeRepo) {
    if (!confirm(`Remove "${repo.name}"? The local clone will be deleted.`)) return;
    try {
      await deleteKnowledgeRepo(isOrg, repo.id);
      refetchRepos();
    } catch {
      // ignore
    }
  }

  async function handleRetryRepo(repo: KnowledgeRepo) {
    try {
      await retryKnowledgeRepo(isOrg, repo.id);
      refetchRepos();
    } catch {
      // ignore
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <GitBranch className={`h-4 w-4 ${accentText}`} />
          <span className="text-[13px] font-medium text-[#e8e0d4]">Git Repos</span>
          {repos.length > 0 && (
            <span className="rounded-full bg-[#232019] px-2 py-0.5 text-[11px] text-[#6b6459]">{repos.length}</span>
          )}
        </div>
        <button
          type="button"
          onClick={() => {
            setAddRepoOpen((v) => !v);
            setAddRepoError(null);
          }}
          className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px] font-medium transition-colors ${accentText} hover:bg-[#232019]`}
        >
          <Plus className="h-3.5 w-3.5" />
          Add
        </button>
      </div>

      {addRepoOpen && (
        <div className="space-y-2 rounded-xl border border-[#2a2520] bg-[#161310] p-3">
          <input
            type="text"
            placeholder="Repository URL (https://github.com/...)"
            value={addRepoUrl}
            onChange={(e) => setAddRepoUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleAddRepo()}
            className="w-full rounded-lg border border-[#2a2520] bg-[#0e0c0a] px-3 py-2 text-[13px] text-[#e8e0d4] placeholder-[#4a443d] outline-none focus:border-[#3a3530]"
          />
          <input
            type="text"
            placeholder="Name (optional, auto-detected from URL)"
            value={addRepoName}
            onChange={(e) => setAddRepoName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleAddRepo()}
            className="w-full rounded-lg border border-[#2a2520] bg-[#0e0c0a] px-3 py-2 text-[13px] text-[#e8e0d4] placeholder-[#4a443d] outline-none focus:border-[#3a3530]"
          />
          {addRepoError && <div className="text-[12px] text-red-400">{addRepoError}</div>}
          <div className="flex gap-2">
            <button
              type="button"
              onClick={handleAddRepo}
              disabled={addRepoLoading || !addRepoUrl.trim()}
              className={`flex-1 rounded-lg py-2 text-[13px] font-medium transition-colors disabled:opacity-50 ${accentBg} ${accentText} hover:opacity-80`}
            >
              {addRepoLoading ? "Cloning..." : "Add Repo"}
            </button>
            <button
              type="button"
              onClick={() => {
                setAddRepoOpen(false);
                setAddRepoError(null);
                setAddRepoUrl("");
                setAddRepoName("");
              }}
              className="rounded-lg px-3 py-2 text-[13px] text-[#6b6459] hover:bg-[#232019] hover:text-[#e8e0d4]"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {repos.length === 0 && !addRepoOpen && (
        <p className="text-[12px] text-[#4a443d]">No repos added yet. Agents will have access to cloned repos.</p>
      )}

      {repos.map((repo) => (
        <div
          key={repo.id}
          className="flex items-center gap-3 rounded-xl border border-[#1e1b18] bg-[#0e0c0a] px-3 py-2.5"
        >
          <GitBranch className="h-4 w-4 shrink-0 text-[#4a443d]" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate text-[13px] font-medium text-[#e8e0d4]">{repo.name}</span>
              <span
                className={`shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
                  repo.status === "ready"
                    ? "bg-emerald-500/10 text-emerald-400"
                    : repo.status === "error"
                      ? "bg-red-500/10 text-red-400"
                      : "bg-amber-500/10 text-amber-400"
                }`}
              >
                {repo.status === "pending" ? "queued" : repo.status}
              </span>
            </div>
            <div className="truncate text-[11px] text-[#4a443d]">{repo.url}</div>
            {repo.status === "error" && repo.error_msg && (
              <div className="mt-1 text-[11px] text-red-400/80">
                {repoErrorHint(repo.error_msg) ?? repo.error_msg}
              </div>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {repo.status === "error" && (
              <button
                type="button"
                onClick={() => handleRetryRepo(repo)}
                className="rounded-lg p-1.5 text-[#4a443d] transition-colors hover:bg-amber-500/10 hover:text-amber-400"
                title="Retry clone"
              >
                <RotateCw className="h-3.5 w-3.5" />
              </button>
            )}
            <button
              type="button"
              onClick={() => handleDeleteRepo(repo)}
              className="rounded-lg p-1.5 text-[#4a443d] transition-colors hover:bg-red-500/10 hover:text-red-400"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}
