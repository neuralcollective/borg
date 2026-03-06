import { useState, useRef, lazy, Suspense } from "react";
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

const DocxViewer = lazy(() => import("./viewers/docx-viewer").then(m => ({ default: m.DocxViewer })));

function formatBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function FileRow({
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
    <div className="border border-zinc-700 rounded-lg p-3 space-y-2">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="font-mono text-sm text-zinc-100 truncate">{file.file_name}</div>
          <div className="text-xs text-zinc-500 mt-0.5">
            {formatBytes(file.size_bytes)} &middot; {new Date(file.created_at).toLocaleDateString()}
          </div>
        </div>
        <div className="flex gap-2 shrink-0">
          {isPreviewable && (
            <button
              onClick={() => onPreview(file)}
              className="text-xs text-blue-400 hover:text-blue-300 px-2 py-1 rounded border border-zinc-700 hover:border-blue-700 transition-colors"
            >
              Preview
            </button>
          )}
          <button
            onClick={() => setEditing((v) => !v)}
            className="text-xs text-zinc-400 hover:text-zinc-200 px-2 py-1 rounded border border-zinc-700 hover:border-zinc-500 transition-colors"
          >
            {editing ? "Cancel" : "Edit"}
          </button>
          <button
            onClick={remove}
            disabled={deleting}
            className="text-xs text-red-400 hover:text-red-300 px-2 py-1 rounded border border-zinc-700 hover:border-red-700 transition-colors disabled:opacity-50"
          >
            {deleting ? "..." : "Delete"}
          </button>
        </div>
      </div>

      {!editing && file.description && (
        <div className="text-sm text-zinc-400">{file.description}</div>
      )}
      {!editing && (
        <div className="flex items-center gap-1.5 flex-wrap">
          <span
            className={cn(
              "text-xs px-1.5 py-0.5 rounded",
              file.inline
                ? "bg-blue-900/50 text-blue-300 border border-blue-700"
                : "bg-zinc-800 text-zinc-400 border border-zinc-700",
            )}
          >
            {file.inline ? "Inline" : "Listed"}
          </span>
          {file.category && file.category !== "general" && (
            <span className={cn("text-xs px-1.5 py-0.5 rounded border",
              file.category === "template" ? "bg-violet-900/50 text-violet-300 border-violet-700"
                : file.category === "clause" ? "bg-emerald-900/50 text-emerald-300 border-emerald-700"
                : "bg-zinc-800 text-zinc-400 border-zinc-700"
            )}>
              {file.category}
            </span>
          )}
          {file.tags && file.tags.split(",").filter(Boolean).map(t => (
            <span key={t.trim()} className="text-xs px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-500 border border-zinc-700">
              {t.trim()}
            </span>
          ))}
        </div>
      )}

      {editing && (
        <div className="space-y-2 pt-1">
          <div>
            <label className="text-xs text-zinc-400 block mb-1">Description</label>
            <input
              type="text"
              value={desc}
              onChange={(e) => setDesc(e.target.value)}
              placeholder="Brief description of this file"
              className="w-full bg-zinc-800 border border-zinc-600 rounded px-2 py-1 text-sm text-zinc-100 focus:outline-none focus:border-zinc-400"
            />
          </div>
          <div>
            <label className="text-xs text-zinc-400 block mb-1">Category</label>
            <select
              value={category}
              onChange={(e) => setCategory(e.target.value)}
              className="w-full bg-zinc-800 border border-zinc-600 rounded px-2 py-1 text-sm text-zinc-100 focus:outline-none focus:border-zinc-400"
            >
              <option value="general">General</option>
              <option value="template">Template</option>
              <option value="clause">Clause</option>
              <option value="reference">Reference</option>
              <option value="policy">Policy</option>
            </select>
          </div>
          <div>
            <label className="text-xs text-zinc-400 block mb-1">Tags (comma-separated)</label>
            <input
              type="text"
              value={tags}
              onChange={(e) => setTags(e.target.value)}
              placeholder="e.g. nda, confidentiality, employment"
              className="w-full bg-zinc-800 border border-zinc-600 rounded px-2 py-1 text-sm text-zinc-100 focus:outline-none focus:border-zinc-400"
            />
          </div>
          <div className="flex items-center gap-2">
            <input
              id={`inline-${file.id}`}
              type="checkbox"
              checked={inline}
              onChange={(e) => setInline(e.target.checked)}
              className="rounded"
            />
            <label htmlFor={`inline-${file.id}`} className="text-sm text-zinc-300">
              Inline (embed content in prompt)
            </label>
          </div>
          <button
            onClick={save}
            disabled={saving}
            className="text-xs bg-zinc-700 hover:bg-zinc-600 text-zinc-100 px-3 py-1.5 rounded transition-colors disabled:opacity-50"
          >
            {saving ? "Saving..." : "Save"}
          </button>
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
  const [description, setDescription] = useState("");
  const [inline, setInline] = useState(false);
  const [uploadCategory, setUploadCategory] = useState("general");
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [previewFile, setPreviewFile] = useState<KnowledgeFile | null>(null);
  const [previewBuffer, setPreviewBuffer] = useState<ArrayBuffer | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);

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

  return (
    <div className="flex flex-col h-full overflow-y-auto p-4 space-y-6 max-w-2xl mx-auto w-full">
      <div>
        <h2 className="text-lg font-semibold text-zinc-100">Knowledge Base</h2>
        <p className="text-sm text-zinc-400 mt-1">
          Files available to all agents at <code className="text-zinc-300">/knowledge/</code>.
          Inline files are embedded directly in the prompt; listed files are mentioned by name.
        </p>
        <div className="mt-3 flex items-center gap-2">
          <input
            type="text"
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              setOffset(0);
            }}
            placeholder="Filter knowledge files"
            className="w-full bg-zinc-800 border border-zinc-600 rounded px-2 py-1.5 text-sm text-zinc-100 focus:outline-none focus:border-zinc-400"
          />
          <span className="shrink-0 text-xs text-zinc-500">{page?.total ?? files.length} files</span>
        </div>
        {page && (
          <div className="mt-2 text-xs text-zinc-500">
            Showing {page.total === 0 ? 0 : page.offset + 1}-{Math.min(page.offset + files.length, page.total)} of {page.total} · {(page.total_bytes / (1024 * 1024)).toFixed(1)} MB total
          </div>
        )}
      </div>

      {/* Upload form */}
      <div className="border border-zinc-700 rounded-lg p-4 space-y-3 bg-zinc-900/50">
        <h3 className="text-sm font-medium text-zinc-200">Upload File</h3>
        <div>
          <input
            ref={fileInputRef}
            type="file"
            onChange={(e) => setSelectedFile(e.target.files?.[0] ?? null)}
            className="block w-full text-sm text-zinc-400 file:mr-3 file:py-1.5 file:px-3 file:rounded file:border file:border-zinc-600 file:bg-zinc-800 file:text-zinc-200 file:text-xs file:cursor-pointer hover:file:bg-zinc-700"
          />
        </div>
        <div>
          <label className="text-xs text-zinc-400 block mb-1">Description</label>
          <input
            type="text"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="What is this file? (shown in prompt)"
            className="w-full bg-zinc-800 border border-zinc-600 rounded px-2 py-1 text-sm text-zinc-100 focus:outline-none focus:border-zinc-400"
          />
        </div>
        <div>
          <label className="text-xs text-zinc-400 block mb-1">Category</label>
          <select
            value={uploadCategory}
            onChange={(e) => setUploadCategory(e.target.value)}
            className="w-full bg-zinc-800 border border-zinc-600 rounded px-2 py-1 text-sm text-zinc-100 focus:outline-none focus:border-zinc-400"
          >
            <option value="general">General</option>
            <option value="template">Template (firm letterhead/styles)</option>
            <option value="clause">Clause Library</option>
            <option value="reference">Reference</option>
            <option value="policy">Policy</option>
          </select>
        </div>
        <div className="flex items-center gap-2">
          <input
            id="upload-inline"
            type="checkbox"
            checked={inline}
            onChange={(e) => setInline(e.target.checked)}
            className="rounded"
          />
          <label htmlFor="upload-inline" className="text-sm text-zinc-300">
            Inline (embed file content in agent prompts)
          </label>
        </div>
        {uploadError && <div className="text-xs text-red-400">{uploadError}</div>}
        <button
          onClick={handleUpload}
          disabled={!selectedFile || uploading}
          className="text-sm bg-zinc-700 hover:bg-zinc-600 text-zinc-100 px-4 py-2 rounded transition-colors disabled:opacity-40"
        >
          {uploading ? "Uploading..." : "Upload"}
        </button>
      </div>

      {/* File list */}
      <div className="space-y-3">
        {isLoading && <div className="text-sm text-zinc-500">Loading...</div>}
        {!isLoading && files.length === 0 && (
          <div className="text-sm text-zinc-500 text-center py-8">
            {page && page.total > 0 ? "No files match the current filter." : "No knowledge files uploaded yet."}
          </div>
        )}
        {files.map((file) => (
          <FileRow
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
        {page && page.total > page.limit && (
          <div className="flex items-center justify-between text-xs text-zinc-500">
            <button
              onClick={() => setOffset((prev) => Math.max(0, prev - page.limit))}
              disabled={page.offset === 0}
              className="rounded border border-zinc-700 px-2 py-1 disabled:opacity-40"
            >
              Prev
            </button>
            <button
              onClick={() => setOffset((prev) => prev + page.limit)}
              disabled={!page.has_more}
              className="rounded border border-zinc-700 px-2 py-1 disabled:opacity-40"
            >
              Next
            </button>
          </div>
        )}
      </div>

      {previewFile && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick={() => { setPreviewFile(null); setPreviewBuffer(null); }}>
          <div className="mx-4 flex max-h-[85vh] w-full max-w-4xl flex-col rounded-lg border border-white/10 bg-zinc-900 shadow-xl" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between border-b border-white/10 px-4 py-3">
              <div>
                <span className="text-sm font-medium text-zinc-200">{previewFile.file_name}</span>
                {previewFile.category === "template" && (
                  <span className="ml-2 rounded bg-violet-900/50 px-1.5 py-0.5 text-[10px] text-violet-300 border border-violet-700">template</span>
                )}
              </div>
              <button onClick={() => { setPreviewFile(null); setPreviewBuffer(null); }} className="text-zinc-500 hover:text-zinc-300">✕</button>
            </div>
            <div className="flex-1 overflow-auto p-4">
              {previewLoading && <div className="text-sm text-zinc-500 text-center py-8">Loading preview...</div>}
              {!previewLoading && previewBuffer && /\.docx$/i.test(previewFile.file_name) && (
                <Suspense fallback={<div className="text-sm text-zinc-500">Loading viewer...</div>}>
                  <DocxViewer buffer={previewBuffer} />
                </Suspense>
              )}
              {!previewLoading && previewBuffer && /\.pdf$/i.test(previewFile.file_name) && (
                <iframe
                  src={URL.createObjectURL(new Blob([previewBuffer], { type: "application/pdf" }))}
                  className="w-full h-[70vh] rounded"
                />
              )}
              {!previewLoading && previewBuffer && /\.(png|jpg|jpeg|gif|svg)$/i.test(previewFile.file_name) && (
                <img
                  src={URL.createObjectURL(new Blob([previewBuffer]))}
                  className="max-w-full max-h-[70vh] mx-auto"
                  alt={previewFile.file_name}
                />
              )}
              {!previewLoading && previewBuffer && /\.(txt|md|csv)$/i.test(previewFile.file_name) && (
                <pre className="whitespace-pre-wrap font-mono text-[12px] text-zinc-300">{new TextDecoder().decode(previewBuffer)}</pre>
              )}
              {!previewLoading && !previewBuffer && (
                <div className="text-sm text-zinc-500 text-center py-8">Failed to load preview</div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
