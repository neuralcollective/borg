import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useModes, useStatus, createTask } from "@/lib/api";
import { repoName } from "@/lib/types";
import { Plus, X } from "lucide-react";

export function TaskCreator() {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [mode, setMode] = useState("swe");
  const [repoPath, setRepoPath] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");
  const queryClient = useQueryClient();
  const { data: modes } = useModes();
  const { data: status } = useStatus();

  const repos = status?.watched_repos ?? [];

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!title.trim()) return;
    setSubmitting(true);
    setError("");
    try {
      await createTask(title.trim(), description.trim(), mode, repoPath || undefined);
      queryClient.invalidateQueries({ queryKey: ["tasks"] });
      setTitle("");
      setDescription("");
      setOpen(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create task");
    } finally {
      setSubmitting(false);
    }
  }

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="inline-flex items-center gap-1.5 rounded-md bg-blue-500/15 px-3 py-1.5 text-[11px] font-medium text-blue-400 ring-1 ring-inset ring-blue-500/20 transition-colors hover:bg-blue-500/25"
      >
        <Plus className="h-3 w-3" />
        New Task
      </button>
    );
  }

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 pt-[15vh]" onClick={() => setOpen(false)}>
      <form
        onClick={(e) => e.stopPropagation()}
        onSubmit={handleSubmit}
        className="w-full max-w-lg rounded-lg border border-white/[0.08] bg-zinc-900 p-5 shadow-2xl"
      >
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-sm font-semibold text-zinc-200">Create Task</h2>
          <button type="button" onClick={() => setOpen(false)} className="text-zinc-500 hover:text-zinc-300">
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-3">
          <input
            autoFocus
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Task title"
            className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 placeholder-zinc-600 outline-none focus:border-blue-500/40"
          />

          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="Description (optional)"
            rows={3}
            className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 placeholder-zinc-600 outline-none focus:border-blue-500/40 resize-none"
          />

          <div className="flex gap-3">
            <div className="flex-1">
              <label className="mb-1 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">Mode</label>
              <select
                value={mode}
                onChange={(e) => setMode(e.target.value)}
                className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 outline-none focus:border-blue-500/40"
              >
                {modes?.map((m) => (
                  <option key={m.name} value={m.name}>{m.label}</option>
                )) ?? <option value="swe">Software Engineering</option>}
              </select>
            </div>

            {repos.length > 1 && (
              <div className="flex-1">
                <label className="mb-1 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">Repository</label>
                <select
                  value={repoPath}
                  onChange={(e) => setRepoPath(e.target.value)}
                  className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 outline-none focus:border-blue-500/40"
                >
                  <option value="">Default</option>
                  {repos.map((r) => (
                    <option key={r.path} value={r.path}>{repoName(r.path)}</option>
                  ))}
                </select>
              </div>
            )}
          </div>
        </div>

        {error && <p className="mt-2 text-[11px] text-red-400">{error}</p>}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={() => setOpen(false)}
            className="rounded-md px-3 py-1.5 text-[12px] text-zinc-400 hover:text-zinc-200"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={submitting || !title.trim()}
            className="rounded-md bg-blue-500/20 px-4 py-1.5 text-[12px] font-medium text-blue-400 ring-1 ring-inset ring-blue-500/20 transition-colors hover:bg-blue-500/30 disabled:opacity-50"
          >
            {submitting ? "Creating..." : "Create"}
          </button>
        </div>
      </form>
    </div>
  );
}
