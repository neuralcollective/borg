import { FileText, X } from "lucide-react";
import { lazy, Suspense, useCallback, useEffect, useState } from "react";

const DocxViewer = lazy(() => import("./viewers/docx-viewer").then((m) => ({ default: m.DocxViewer })));
const XlsxViewer = lazy(() => import("./viewers/xlsx-viewer").then((m) => ({ default: m.XlsxViewer })));
const PptxViewer = lazy(() => import("./viewers/pptx-viewer").then((m) => ({ default: m.PptxViewer })));
const PdfViewer = lazy(() => import("./viewers/pdf-viewer").then((m) => ({ default: m.PdfViewer })));

type ViewerType = "docx" | "xlsx" | "pptx" | "pdf" | "image" | "text";

function pickViewer(name: string, mime?: string): ViewerType | null {
  const lower = name.toLowerCase();
  if (mime?.includes("pdf") || lower.endsWith(".pdf")) return "pdf";
  if (mime?.includes("wordprocessingml") || lower.endsWith(".docx")) return "docx";
  if (mime?.includes("spreadsheetml") || lower.endsWith(".xlsx")) return "xlsx";
  if (mime?.includes("presentationml") || lower.endsWith(".pptx")) return "pptx";
  if (/\.(png|jpg|jpeg|gif|svg|webp)$/i.test(lower) || mime?.startsWith("image/")) return "image";
  if (
    /\.(txt|md|csv|json|log|xml|yaml|yml|toml|ini|cfg|conf|sh|py|js|ts|rs|go|rb|html|css)$/i.test(lower) ||
    mime?.startsWith("text/")
  )
    return "text";
  return null;
}

/** Check if a file can be previewed, works with or without mime_type */
export function isPreviewable(file: { file_name: string; mime_type?: string }): boolean {
  return pickViewer(file.file_name, file.mime_type) !== null;
}

function Spinner() {
  return (
    <div className="flex h-48 items-center justify-center">
      <div className="h-6 w-6 animate-spin rounded-full border-2 border-zinc-700 border-t-zinc-400" />
    </div>
  );
}

export interface PreviewableFile {
  id: number;
  file_name: string;
  mime_type?: string;
}

export function FilePreviewModal({
  file,
  fetchContent,
  onClose,
  isActive,
}: {
  file: PreviewableFile;
  fetchContent: (fileId: number) => Promise<ArrayBuffer>;
  onClose: () => void;
  isActive?: boolean;
}) {
  const [buffer, setBuffer] = useState<ArrayBuffer | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const viewerType = pickViewer(file.file_name, file.mime_type);

  const loadContent = useCallback(() => {
    setBuffer(null);
    setLoading(true);
    setError(null);
    fetchContent(file.id)
      .then(setBuffer)
      .catch((e) => setError(e.message || "Failed to load file"))
      .finally(() => setLoading(false));
  }, [fetchContent, file.id]);

  useEffect(() => {
    loadContent();
  }, [loadContent]);

  useEffect(() => {
    const handleEsc = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleEsc);
    return () => window.removeEventListener("keydown", handleEsc);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[85vh] w-full max-w-5xl flex-col overflow-hidden rounded-2xl border border-white/[0.08] bg-[#0e0e10] shadow-2xl mx-4">
        <div className="flex items-center justify-between border-b border-white/[0.07] px-5 py-4">
          <div className="flex items-center gap-3 min-w-0">
            <FileText className="h-4 w-4 shrink-0 text-zinc-500" />
            <span className="truncate text-[14px] font-medium text-zinc-200">{file.file_name}</span>
            {isActive && (
              <span className="flex items-center gap-1.5 shrink-0 rounded-full bg-amber-500/10 px-2.5 py-1 ring-1 ring-amber-500/20">
                <span className="h-2 w-2 rounded-full bg-amber-400 animate-pulse" />
                <span className="text-[11px] font-medium text-amber-400">Agent editing</span>
              </span>
            )}
          </div>
          <button
            onClick={onClose}
            className="shrink-0 rounded-lg p-2 text-zinc-500 transition-colors hover:bg-white/[0.06] hover:text-zinc-300"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-auto p-5">
          {loading && <Spinner />}
          {error && (
            <div className="flex h-48 flex-col items-center justify-center gap-3">
              <span className="text-[13px] text-red-400">{error}</span>
              <button
                onClick={loadContent}
                className="rounded-lg border border-white/[0.07] px-4 py-2 text-[12px] text-zinc-400 transition-colors hover:border-white/[0.12] hover:text-zinc-300"
              >
                Retry
              </button>
            </div>
          )}
          {!loading && !error && buffer && (
            <Suspense fallback={<Spinner />}>
              {viewerType === "docx" && <DocxViewer buffer={buffer} />}
              {viewerType === "xlsx" && <XlsxViewer buffer={buffer} />}
              {viewerType === "pptx" && <PptxViewer buffer={buffer} />}
              {viewerType === "pdf" && <PdfViewer buffer={buffer} />}
              {viewerType === "image" && (
                <img
                  src={URL.createObjectURL(new Blob([buffer]))}
                  className="max-w-full max-h-[70vh] mx-auto rounded-lg"
                  alt={file.file_name}
                />
              )}
              {viewerType === "text" && (
                <pre className="whitespace-pre-wrap font-mono text-[13px] leading-relaxed text-zinc-300">
                  {new TextDecoder().decode(buffer)}
                </pre>
              )}
              {!viewerType && (
                <div className="flex h-48 items-center justify-center text-[13px] text-zinc-500">
                  Preview not available for this file type
                </div>
              )}
            </Suspense>
          )}
        </div>
      </div>
    </div>
  );
}
