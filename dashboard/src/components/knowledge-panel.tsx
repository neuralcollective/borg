import { useState, useRef, lazy, Suspense, useCallback } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useKnowledgeFiles,
  uploadKnowledgeFile,
  updateKnowledgeFile,
  deleteKnowledgeFile,
  fetchKnowledgeContent,
} from "@/lib/api";
import type { KnowledgeFile } from "@/lib/types";
import { cn } from "@/lib/utils";
import { Search, Upload, FileText, Eye, Pencil, Trash2, ChevronLeft, ChevronRight, BookOpen, X } from "lucide-react";

const DocxViewer = lazy(() => import("./viewers/docx-viewer").then(m => ({ default: m.DocxViewer })));

function formatBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

const CATEGORIES = [
  { value: "general", label: "General" },
  { value: "template", label: "Template" },
  { value: "clause", label: "Clause Library" },
  { value: "reference", label: "Reference" },
  { value: "policy", label: "Policy" },
];

const categoryColors: Record<string, string> = {
  template: "bg-violet-500/15 text-violet-300 ring-violet-500/20",
  clause: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/20",
  reference: "bg-cyan-500/15 text-cyan-300 ring-cyan-500/20",
  policy: "bg-amber-500/15 text-amber-300 ring-amber-500/20",
};

function FileCard({
  file,
  onDeleted,
  onUpdated,
  onPreview,
}: {
  file: KnowledgeFile;
  onDeleted: () => void;
  onUpdated: () => void;
  onPreview: (file: KnowledgeFile) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [desc, setDesc] = useState(file.description);
  const [inline, setInline] = useState(file.inline);
  const [category, setCategory] = useState(file.category || "general");
  const [tags, setTags] = useState(file.tags || "");
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const isPreviewable = /\.(docx|pdf|png|jpg|jpeg|gif|svg|txt|md|csv)$/i.test(file.file_name);

  async function save() {
    setSaving(true);
    try {
      await updateKnowledgeFile(file.id, { description: desc, inline, category, tags });
      onUpdated();
      setEditing(false);
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!confirm(`Delete "${file.file_name}"?`)) return;
    setDeleting(true);
    try {
      await deleteKnowledgeFile(file.id);
      onDeleted();
    } finally {
      setDeleting(false);
    }
  }

  return (
    <div className="group rounded-xl border border-white/[0.07] bg-white/[0.02] p-4 transition-colors hover:border-white/[0.1] hover:bg-white/[0.03]">
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-start gap-3 min-w-0">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-white/[0.04] ring-1 ring-white/[0.06]">
            <FileText className="h-4 w-4 text-zinc-500" />
          </div>
          <div className="min-w-0">
            <div className="text-[13px] font-medium text-zinc-100 truncate">{file.file_name}</div>
            <div className="mt-0.5 text-[12px] text-zinc-500">
              {formatBytes(file.size_bytes)} · {new Date(file.created_at).toLocaleDateString()}
            </div>
          </div>
        </div>
        <div className="flex gap-1.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
          {isPreviewable && (
            <button
              onClick={() => onPreview(file)}
              className="rounded-lg p-2 text-zinc-500 transition-colors hover:bg-white/[0.06] hover:text-blue-400"
              title="Preview"
            >
              <Eye className="h-3.5 w-3.5" />
            </button>
          )}
          <button
            onClick={() => setEditing((v) => !v)}
            className="rounded-lg p-2 text-zinc-500 transition-colors hover:bg-white/[0.06] hover:text-zinc-300"
            title="Edit"
          >
            <Pencil className="h-3.5 w-3.5" />
          </button>
          <button
            onClick={remove}
            disabled={deleting}
            className="rounded-lg p-2 text-zinc-500 transition-colors hover:bg-red-500/10 hover:text-red-400 disabled:opacity-50"
            title="Delete"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {!editing && file.description && (
        <p className="mt-2 text-[13px] leading-relaxed text-zinc-400">{file.description}</p>
      )}

      {!editing && (
        <div className="mt-3 flex items-center gap-2 flex-wrap">
          <span
            className={cn(
              "rounded-full px-2.5 py-0.5 text-[11px] font-medium ring-1 ring-inset",
              file.inline
                ? "bg-blue-500/15 text-blue-300 ring-blue-500/20"
                : "bg-white/[0.04] text-zinc-400 ring-white/[0.08]"
            )}
          >
            {file.inline ? "Inline" : "Listed"}
          </span>
          {file.category && file.category !== "general" && (
            <span className={cn("rounded-full px-2.5 py-0.5 text-[11px] font-medium ring-1 ring-inset",
              categoryColors[file.category] ?? "bg-white/[0.04] text-zinc-400 ring-white/[0.08]"
            )}>
              {file.category}
            </span>
          )}
          {file.tags && file.tags.split(",").filter(Boolean).map(t => (
            <span key={t.trim()} className="rounded-full bg-white/[0.04] px-2 py-0.5 text-[11px] text-zinc-500 ring-1 ring-inset ring-white/[0.06]">
              {t.trim()}
            </span>
          ))}
        </div>
      )}

      {editing && (
        <div className="mt-4 space-y-3">
          <div>
            <label className="text-[12px] font-medium text-zinc-400 block mb-1.5">Description</label>
            <input
              type="text"
              value={desc}
              onChange={(e) => setDesc(e.target.value)}
              placeholder="Brief description of this file"
              className="w-full rounded-xl border border-white/[0.07] bg-white/[0.04] px-4 py-2.5 text-[13px] text-zinc-100 outline-none focus:border-white/[0.15] placeholder:text-zinc-600"
            />
          </div>
          <div className="flex gap-3">
            <div className="flex-1">
              <label className="text-[12px] font-medium text-zinc-400 block mb-1.5">Category</label>
              <select
                value={category}
                onChange={(e) => setCategory(e.target.value)}
                className="w-full rounded-xl border border-white/[0.07] bg-white/[0.04] px-4 py-2.5 text-[13px] text-zinc-100 outline-none focus:border-white/[0.15]"
              >
                {CATEGORIES.map(c => <option key={c.value} value={c.value}>{c.label}</option>)}
              </select>
            </div>
            <div className="flex-1">
              <label className="text-[12px] font-medium text-zinc-400 block mb-1.5">Tags</label>
              <input
                type="text"
                value={tags}
                onChange={(e) => setTags(e.target.value)}
                placeholder="comma-separated"
                className="w-full rounded-xl border border-white/[0.07] bg-white/[0.04] px-4 py-2.5 text-[13px] text-zinc-100 outline-none focus:border-white/[0.15] placeholder:text-zinc-600"
              />
            </div>
          </div>
          <label className="flex items-center gap-2.5 cursor-pointer">
            <input
              type="checkbox"
              checked={inline}
              onChange={(e) => setInline(e.target.checked)}
              className="rounded"
            />
            <span className="text-[13px] text-zinc-300">Inline (embed content in agent prompts)</span>
          </label>
          <div className="flex items-center gap-2 pt-1">
            <button
              onClick={save}
              disabled={saving}
              className="rounded-lg bg-blue-500/20 px-4 py-2 text-[13px] font-medium text-blue-300 transition-colors hover:bg-blue-500/30 disabled:opacity-50"
            >
              {saving ? "Saving..." : "Save Changes"}
            </button>
            <button
              onClick={() => { setEditing(false); setDesc(file.description); setInline(file.inline); setCategory(file.category || "general"); setTags(file.tags || ""); }}
              className="rounded-lg px-4 py-2 text-[13px] text-zinc-500 transition-colors hover:text-zinc-300"
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export function KnowledgePanel() {
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const { data: page, isLoading } = useKnowledgeFiles({ limit: 50, offset, q: search });
  const files = page?.files ?? [];
  const queryClient = useQueryClient();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const dropRef = useRef<HTMLDivElement>(null);
  const [description, setDescription] = useState("");
  const [inline, setInline] = useState(false);
  const [uploadCategory, setUploadCategory] = useState("general");
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [previewFile, setPreviewFile] = useState<KnowledgeFile | null>(null);
  const [previewBuffer, setPreviewBuffer] = useState<ArrayBuffer | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [dragOver, setDragOver] = useState(false);

  function invalidate() {
    queryClient.invalidateQueries({ queryKey: ["knowledge"] });
  }

  async function handleUpload() {
    if (!selectedFile) return;
    setUploading(true);
    setUploadError(null);
    try {
      await uploadKnowledgeFile(selectedFile, description, inline, uploadCategory !== "general" ? uploadCategory : undefined);
      setSelectedFile(null);
      setDescription("");
      setInline(false);
      setUploadCategory("general");
      if (fileInputRef.current) fileInputRef.current.value = "";
      invalidate();
    } catch (e) {
      setUploadError(e instanceof Error ? e.message : "Upload failed");
    } finally {
      setUploading(false);
    }
  }

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const file = e.dataTransfer.files[0];
    if (file) setSelectedFile(file);
  }, []);

  return (
    <div className="flex flex-col h-full overflow-y-auto">
      <div className="mx-auto w-full max-w-3xl px-6 py-8 space-y-8">
        {/* Header */}
        <div>
          <div className="flex items-center gap-3 mb-2">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-white/[0.04] ring-1 ring-white/[0.06]">
              <BookOpen className="h-5 w-5 text-zinc-500" />
            </div>
            <div>
              <h2 className="text-[18px] font-semibold text-zinc-100">Knowledge Base</h2>
              <p className="text-[13px] text-zinc-500">
                Files available to all agents at <code className="rounded bg-white/[0.06] px-1.5 py-0.5 text-[12px] text-zinc-300">/knowledge/</code>
              </p>
            </div>
          </div>
        </div>

        {/* Search & stats */}
        <div className="flex items-center gap-3">
          <div className="relative flex-1">
            <Search className="pointer-events-none absolute left-3.5 top-1/2 h-4 w-4 -translate-y-1/2 text-zinc-600" />
            <input
              type="text"
              value={search}
              onChange={(e) => { setSearch(e.target.value); setOffset(0); }}
              placeholder="Search knowledge files..."
              className="w-full rounded-xl border border-white/[0.07] bg-white/[0.03] py-2.5 pl-10 pr-4 text-[14px] text-zinc-100 outline-none transition-colors focus:border-white/[0.15] placeholder:text-zinc-600"
            />
          </div>
          <div className="text-[12px] text-zinc-500 tabular-nums whitespace-nowrap">
            {page?.total ?? files.length} files
            {page && <span className="ml-1 text-zinc-600">· {(page.total_bytes / (1024 * 1024)).toFixed(1)} MB</span>}
          </div>
        </div>

        {/* Upload area */}
        <div
          ref={dropRef}
          onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
          onDragLeave={() => setDragOver(false)}
          onDrop={handleDrop}
          className={cn(
            "rounded-xl border-2 border-dashed p-6 transition-colors",
            dragOver
              ? "border-blue-500/40 bg-blue-500/[0.04]"
              : "border-white/[0.07] bg-white/[0.02]"
          )}
        >
          <div className="flex flex-col items-center gap-3 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-white/[0.04]">
              <Upload className="h-5 w-5 text-zinc-500" />
            </div>
            {selectedFile ? (
              <div className="flex items-center gap-2">
                <FileText className="h-4 w-4 text-blue-400" />
                <span className="text-[13px] text-zinc-200">{selectedFile.name}</span>
                <button onClick={() => { setSelectedFile(null); if (fileInputRef.current) fileInputRef.current.value = ""; }} className="rounded p-0.5 text-zinc-500 hover:text-zinc-300">
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            ) : (
              <>
                <div>
                  <p className="text-[14px] font-medium text-zinc-300">Drop a file here or <button onClick={() => fileInputRef.current?.click()} className="text-blue-400 hover:text-blue-300">browse</button></p>
                  <p className="mt-1 text-[12px] text-zinc-600">Supports any file type. Inline files are embedded in agent prompts.</p>
                </div>
              </>
            )}
            <input
              ref={fileInputRef}
              type="file"
              onChange={(e) => setSelectedFile(e.target.files?.[0] ?? null)}
              className="hidden"
            />
          </div>

          {selectedFile && (
            <div className="mt-4 space-y-3 border-t border-white/[0.06] pt-4">
              <div className="flex gap-3">
                <div className="flex-1">
                  <label className="text-[12px] font-medium text-zinc-400 block mb-1.5">Description</label>
                  <input
                    type="text"
                    value={description}
                    onChange={(e) => setDescription(e.target.value)}
                    placeholder="What is this file?"
                    className="w-full rounded-xl border border-white/[0.07] bg-white/[0.04] px-4 py-2.5 text-[13px] text-zinc-100 outline-none focus:border-white/[0.15] placeholder:text-zinc-600"
                  />
                </div>
                <div className="w-40">
                  <label className="text-[12px] font-medium text-zinc-400 block mb-1.5">Category</label>
                  <select
                    value={uploadCategory}
                    onChange={(e) => setUploadCategory(e.target.value)}
                    className="w-full rounded-xl border border-white/[0.07] bg-white/[0.04] px-3 py-2.5 text-[13px] text-zinc-100 outline-none focus:border-white/[0.15]"
                  >
                    {CATEGORIES.map(c => <option key={c.value} value={c.value}>{c.label}</option>)}
                  </select>
                </div>
              </div>
              <div className="flex items-center justify-between">
                <label className="flex items-center gap-2.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={inline}
                    onChange={(e) => setInline(e.target.checked)}
                    className="rounded"
                  />
                  <span className="text-[13px] text-zinc-300">Inline in prompts</span>
                </label>
                <button
                  onClick={handleUpload}
                  disabled={uploading}
                  className="rounded-lg bg-blue-500 px-5 py-2 text-[13px] font-medium text-white transition-colors hover:bg-blue-400 disabled:opacity-50 shadow-lg shadow-blue-500/20"
                >
                  {uploading ? "Uploading..." : "Upload"}
                </button>
              </div>
              {uploadError && <p className="text-[12px] text-red-400">{uploadError}</p>}
            </div>
          )}
        </div>

        {/* File list */}
        <div className="space-y-3">
          {isLoading && (
            <div className="flex items-center justify-center py-12">
              <div className="h-6 w-6 animate-spin rounded-full border-2 border-zinc-700 border-t-zinc-400" />
            </div>
          )}
          {!isLoading && files.length === 0 && (
            <div className="flex flex-col items-center py-16 text-center">
              <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-white/[0.04] ring-1 ring-white/[0.06]">
                <FileText className="h-6 w-6 text-zinc-600" />
              </div>
              <p className="text-[14px] text-zinc-400">
                {page && page.total > 0 ? "No files match your search" : "No knowledge files yet"}
              </p>
              <p className="mt-1 text-[12px] text-zinc-600">
                {page && page.total > 0 ? "Try a different search term" : "Upload files to make them available to agents"}
              </p>
            </div>
          )}
          {files.map((file) => (
            <FileCard
              key={file.id}
              file={file}
              onDeleted={invalidate}
              onUpdated={invalidate}
              onPreview={async (f) => {
                setPreviewFile(f);
                setPreviewLoading(true);
                try {
                  const buf = await fetchKnowledgeContent(f.id);
                  setPreviewBuffer(buf);
                } catch {
                  setPreviewBuffer(null);
                } finally {
                  setPreviewLoading(false);
                }
              }}
            />
          ))}

          {/* Pagination */}
          {page && page.total > page.limit && (
            <div className="flex items-center justify-between pt-2">
              <span className="text-[12px] text-zinc-500">
                {page.total === 0 ? 0 : page.offset + 1}–{Math.min(page.offset + files.length, page.total)} of {page.total}
              </span>
              <div className="flex gap-2">
                <button
                  onClick={() => setOffset((prev) => Math.max(0, prev - page.limit))}
                  disabled={page.offset === 0}
                  className="flex items-center gap-1 rounded-lg border border-white/[0.07] px-3 py-1.5 text-[12px] text-zinc-400 transition-colors hover:border-white/[0.12] hover:text-zinc-300 disabled:opacity-40"
                >
                  <ChevronLeft className="h-3.5 w-3.5" /> Prev
                </button>
                <button
                  onClick={() => setOffset((prev) => prev + page.limit)}
                  disabled={!page.has_more}
                  className="flex items-center gap-1 rounded-lg border border-white/[0.07] px-3 py-1.5 text-[12px] text-zinc-400 transition-colors hover:border-white/[0.12] hover:text-zinc-300 disabled:opacity-40"
                >
                  Next <ChevronRight className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Preview modal */}
      {previewFile && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm" onClick={() => { setPreviewFile(null); setPreviewBuffer(null); }}>
          <div className="mx-4 flex max-h-[85vh] w-full max-w-4xl flex-col rounded-2xl border border-white/[0.08] bg-[#0e0e10] shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between border-b border-white/[0.07] px-5 py-4">
              <div className="flex items-center gap-3">
                <FileText className="h-4 w-4 text-zinc-500" />
                <span className="text-[14px] font-medium text-zinc-200">{previewFile.file_name}</span>
                {previewFile.category === "template" && (
                  <span className="rounded-full bg-violet-500/15 px-2 py-0.5 text-[11px] font-medium text-violet-300 ring-1 ring-inset ring-violet-500/20">template</span>
                )}
              </div>
              <button onClick={() => { setPreviewFile(null); setPreviewBuffer(null); }} className="rounded-lg p-2 text-zinc-500 transition-colors hover:bg-white/[0.06] hover:text-zinc-300">
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="flex-1 overflow-auto p-5">
              {previewLoading && (
                <div className="flex items-center justify-center py-12">
                  <div className="h-6 w-6 animate-spin rounded-full border-2 border-zinc-700 border-t-zinc-400" />
                </div>
              )}
              {!previewLoading && previewBuffer && /\.docx$/i.test(previewFile.file_name) && (
                <Suspense fallback={<div className="flex items-center justify-center py-12"><div className="h-6 w-6 animate-spin rounded-full border-2 border-zinc-700 border-t-zinc-400" /></div>}>
                  <DocxViewer buffer={previewBuffer} />
                </Suspense>
              )}
              {!previewLoading && previewBuffer && /\.pdf$/i.test(previewFile.file_name) && (
                <iframe
                  src={URL.createObjectURL(new Blob([previewBuffer], { type: "application/pdf" }))}
                  className="w-full h-[70vh] rounded-lg"
                />
              )}
              {!previewLoading && previewBuffer && /\.(png|jpg|jpeg|gif|svg)$/i.test(previewFile.file_name) && (
                <img
                  src={URL.createObjectURL(new Blob([previewBuffer]))}
                  className="max-w-full max-h-[70vh] mx-auto rounded-lg"
                  alt={previewFile.file_name}
                />
              )}
              {!previewLoading && previewBuffer && /\.(txt|md|csv)$/i.test(previewFile.file_name) && (
                <pre className="whitespace-pre-wrap font-mono text-[13px] leading-relaxed text-zinc-300">{new TextDecoder().decode(previewBuffer)}</pre>
              )}
              {!previewLoading && !previewBuffer && (
                <div className="flex flex-col items-center py-12 text-center">
                  <p className="text-[14px] text-zinc-400">Failed to load preview</p>
                  <p className="mt-1 text-[12px] text-zinc-600">The file may be too large or in an unsupported format</p>
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
