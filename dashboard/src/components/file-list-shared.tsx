import { Download, Eye, FileText, Search, Trash2, Upload } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { uploadProjectFiles, useProjectFiles } from "@/lib/api";
import { cn, formatFileSize } from "@/lib/utils";
import { FilePreviewModal, isPreviewable, type PreviewableFile } from "./file-preview-modal";

// ── Shared file shape for generic components ─────────────────────────────────

export interface GenericFile {
  id: number;
  file_name: string;
  size_bytes: number;
}

// ── Download helper ──────────────────────────────────────────────────────────

export async function downloadFile(
  fetchContent: (id: number) => Promise<ArrayBuffer>,
  file: GenericFile,
) {
  const buf = await fetchContent(file.id);
  const url = URL.createObjectURL(new Blob([buf]));
  const a = document.createElement("a");
  a.href = url;
  a.download = file.file_name.split("/").pop() || file.file_name;
  a.click();
  URL.revokeObjectURL(url);
}

// ── useFileList hook ─────────────────────────────────────────────────────────

export function useFileList(projectId: number | null) {
  const [fileSearch, setFileSearch] = useState("");
  const [pageSize, setPageSize] = useState(20);
  const [filePageStack, setFilePageStack] = useState<Array<{ cursor: string | null; offset: number }>>([
    { cursor: null, offset: 0 },
  ]);
  const currentFilePage = filePageStack[filePageStack.length - 1] ?? { cursor: null, offset: 0 };

  const {
    data: filePage,
    refetch: refetchFiles,
    isFetching: filesLoading,
  } = useProjectFiles(projectId, {
    limit: pageSize,
    offset: currentFilePage.offset,
    cursor: currentFilePage.cursor,
    q: fileSearch,
  });
  const files = filePage?.items ?? [];

  useEffect(() => {
    setFilePageStack([{ cursor: null, offset: 0 }]);
    setFileSearch("");
  }, []);

  const resetPagination = useCallback(() => {
    setFilePageStack([{ cursor: null, offset: 0 }]);
  }, []);

  return {
    fileSearch,
    setFileSearch,
    pageSize,
    setPageSize,
    filePageStack,
    setFilePageStack,
    currentFilePage,
    filePage,
    files,
    filesLoading,
    refetchFiles,
    resetPagination,
  };
}

// ── FileUploadArea ───────────────────────────────────────────────────────────

export function FileUploadArea({
  projectId,
  onUploaded,
  onUploadFiles,
  subtitle,
}: {
  projectId?: number;
  onUploaded: () => void;
  onUploadFiles?: (files: File[]) => Promise<void>;
  subtitle?: string;
}) {
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  async function handleUpload(selected: FileList | File[] | null) {
    if (!selected || uploading) return;
    const list = selected instanceof FileList ? Array.from(selected) : selected;
    if (list.length === 0) return;
    setUploading(true);
    setUploadError(null);
    try {
      if (onUploadFiles) {
        await onUploadFiles(list);
      } else if (projectId != null) {
        await uploadProjectFiles(projectId, list);
      }
      onUploaded();
      if (fileInputRef.current) fileInputRef.current.value = "";
    } catch (err) {
      const msg = err instanceof Error ? err.message : "upload failed";
      if (msg === "403") {
        setUploadError("Privileged uploads are only allowed after entering Phase 2.");
      } else {
        setUploadError(`Upload failed (${msg}).`);
      }
    } finally {
      setUploading(false);
    }
  }

  return (
    <div
      onDragOver={(e) => {
        e.preventDefault();
        setDragOver(true);
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragOver(false);
        void handleUpload(Array.from(e.dataTransfer.files));
      }}
      onClick={() => fileInputRef.current?.click()}
      className={cn(
        "rounded-xl border-2 border-dashed p-4 transition-colors cursor-pointer",
        dragOver
          ? "border-amber-500/40 bg-amber-500/[0.04]"
          : "border-[#2a2520] bg-[#151412] hover:border-amber-500/20",
      )}
    >
      <div className="flex items-center gap-3">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[#1c1a17]">
          <Upload className="h-4 w-4 text-[#6b6459]" />
        </div>
        <div>
          <p className="text-[13px] font-medium text-[#e8e0d4]">
            Drop files here or <span className="text-amber-400">browse</span>
          </p>
          <p className="mt-0.5 text-[11px] text-[#6b6459]">{subtitle ?? "Upload source documents"}</p>
        </div>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          onChange={(e) => void handleUpload(e.target.files)}
          disabled={uploading}
          className="hidden"
        />
      </div>
      {uploading && <p className="mt-2 text-center text-[12px] text-amber-400">Uploading...</p>}
      {uploadError && <p className="mt-2 text-center text-[12px] text-red-400">{uploadError}</p>}
    </div>
  );
}

// ── FileSearchBar ────────────────────────────────────────────────────────────

export function FileSearchBar({
  value,
  onChange,
  placeholder,
  className,
  stats,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  className?: string;
  stats?: React.ReactNode;
}) {
  return (
    <div className={cn("flex items-center gap-3", className)}>
      <div className="relative flex-1">
        <Search className="pointer-events-none absolute left-3.5 top-1/2 h-4 w-4 -translate-y-1/2 text-[#6b6459]" />
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder ?? "Search files..."}
          className="w-full rounded-xl border border-[#2a2520] bg-[#151412] py-2.5 pl-10 pr-4 text-[14px] text-[#e8e0d4] outline-none placeholder:text-[#6b6459] focus:border-amber-500/30"
        />
      </div>
      {stats && <div className="shrink-0 text-[12px] text-[#6b6459] tabular-nums whitespace-nowrap">{stats}</div>}
    </div>
  );
}

// ── FileListPagination ───────────────────────────────────────────────────────

export function FileListPagination({
  filePage,
  currentOffset,
  fileCount,
  pageSize,
  onPageSizeChange,
  canGoPrev,
  onPrev,
  canGoNext,
  onNext,
  actions,
}: {
  filePage: { total: number; has_more: boolean; next_cursor?: string | null };
  currentOffset: number;
  fileCount: number;
  pageSize: number;
  onPageSizeChange: (size: number) => void;
  canGoPrev: boolean;
  onPrev: () => void;
  canGoNext: boolean;
  onNext: () => void;
  actions?: React.ReactNode;
}) {
  if (filePage.total <= fileCount) return null;

  return (
    <div className="flex items-center justify-between gap-3 text-[11px] text-[#6b6459]">
      <span>
        {filePage.total === 0 ? 0 : currentOffset + 1}–{Math.min(currentOffset + fileCount, filePage.total)} of{" "}
        {filePage.total}
      </span>
      <div className="flex items-center gap-2">
        <select
          value={pageSize}
          onChange={(e) => onPageSizeChange(Number(e.target.value))}
          className="rounded-lg border border-[#2a2520] bg-[#151412] px-2 py-1.5 text-[12px] text-[#9c9486] outline-none"
        >
          {[20, 50, 100].map((s) => (
            <option key={s} value={s}>
              {s} / page
            </option>
          ))}
        </select>
        <button
          onClick={onPrev}
          disabled={!canGoPrev}
          className="rounded-lg border border-[#2a2520] px-3 py-1.5 text-[12px] text-[#9c9486] disabled:opacity-40 hover:border-amber-900/30"
        >
          Prev
        </button>
        <button
          onClick={onNext}
          disabled={!canGoNext}
          className="rounded-lg border border-[#2a2520] px-3 py-1.5 text-[12px] text-[#9c9486] disabled:opacity-40 hover:border-amber-900/30"
        >
          Next
        </button>
        {actions}
      </div>
    </div>
  );
}

// ── FileListItem ─────────────────────────────────────────────────────────────

export function FileListItem({
  file,
  index,
  isActive,
  onClick,
  onDownload,
  onDelete,
  extraActions,
  extraBadges,
}: {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  file: GenericFile & { [key: string]: any };
  index?: number;
  isActive?: boolean;
  onClick?: () => void;
  onDownload?: () => void;
  onDelete?: () => void;
  extraActions?: React.ReactNode;
  extraBadges?: React.ReactNode;
}) {
  const canPreview = isPreviewable(file);

  return (
    <div
      onClick={() => onClick?.()}
      className={cn(
        "group flex items-center gap-2.5 rounded-xl border px-3 py-2 transition-colors hover:border-amber-900/30",
        isActive ? "border-amber-500/30 bg-[#1a1814]" : "border-[#2a2520] bg-[#151412]",
        (canPreview || onClick) && "cursor-pointer",
      )}
    >
      <div
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg",
          isActive ? "bg-amber-500/10 ring-1 ring-amber-500/30" : "bg-[#1c1a17] ring-1 ring-amber-900/20",
        )}
      >
        <FileText className={cn("h-3.5 w-3.5", isActive ? "text-amber-400" : "text-[#6b6459]")} />
      </div>
      {index != null && (
        <span className="shrink-0 text-[11px] text-[#6b6459] tabular-nums w-[32px] text-right">{index}</span>
      )}
      <span className="shrink-0 text-[11px] text-[#6b6459] tabular-nums w-[52px]">
        {formatFileSize(file.size_bytes)}
      </span>
      <div className="min-w-0 flex-1 flex items-center gap-2">
        <span className="text-[13px] font-medium text-[#e8e0d4] truncate">{file.file_name}</span>
        {isActive && (
          <span className="flex items-center gap-1 shrink-0">
            <span className="h-2 w-2 rounded-full bg-amber-400 animate-pulse" />
            <span className="text-[10px] text-amber-400">editing</span>
          </span>
        )}
      </div>
      <div className="flex items-center gap-1.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
        {extraBadges}
        {onDownload && (
          <button
            onClick={(e) => { e.stopPropagation(); onDownload(); }}
            className="rounded-lg p-2 text-[#6b6459] transition-colors hover:bg-amber-500/10 hover:text-amber-400"
            title="Download"
          >
            <Download className="h-3.5 w-3.5" />
          </button>
        )}
        {onDelete && (
          <button
            onClick={(e) => { e.stopPropagation(); onDelete(); }}
            className="rounded-lg p-2 text-[#6b6459] transition-colors hover:bg-red-500/10 hover:text-red-400"
            title="Delete"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        )}
        {extraActions}
        {canPreview && !onDownload && !onDelete && !extraActions && (
          <Eye className="h-3.5 w-3.5 text-[#6b6459]" />
        )}
      </div>
    </div>
  );
}

// ── FilePreview wrapper ──────────────────────────────────────────────────────

export function useFilePreview() {
  const [previewFile, setPreviewFile] = useState<PreviewableFile | null>(null);
  return { previewFile, setPreviewFile };
}

export function FilePreviewWrapper({
  file,
  fetchContent,
  onClose,
  isActive,
}: {
  file: PreviewableFile | null;
  fetchContent: (fileId: number) => Promise<ArrayBuffer>;
  onClose: () => void;
  isActive?: boolean;
}) {
  if (!file) return null;
  return <FilePreviewModal file={file} fetchContent={fetchContent} onClose={onClose} isActive={isActive} />;
}

export { isPreviewable, formatFileSize };
