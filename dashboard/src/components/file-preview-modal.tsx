import { useEffect, useState, lazy, Suspense } from "react";
import { X } from "lucide-react";
import { fetchProjectFileContent } from "@/lib/api";
import type { ProjectFile } from "@/lib/types";

const DocxViewer = lazy(() =>
  import("./viewers/docx-viewer").then((m) => ({ default: m.DocxViewer }))
);
const XlsxViewer = lazy(() =>
  import("./viewers/xlsx-viewer").then((m) => ({ default: m.XlsxViewer }))
);
const PptxViewer = lazy(() =>
  import("./viewers/pptx-viewer").then((m) => ({ default: m.PptxViewer }))
);
const PdfViewer = lazy(() =>
  import("./viewers/pdf-viewer").then((m) => ({ default: m.PdfViewer }))
);

type ViewerType = "docx" | "xlsx" | "pptx" | "pdf";

function pickViewer(mime: string, name: string): ViewerType | null {
  const lower = name.toLowerCase();
  if (mime.includes("pdf") || lower.endsWith(".pdf")) return "pdf";
  if (mime.includes("wordprocessingml") || lower.endsWith(".docx")) return "docx";
  if (mime.includes("spreadsheetml") || lower.endsWith(".xlsx")) return "xlsx";
  if (mime.includes("presentationml") || lower.endsWith(".pptx")) return "pptx";
  return null;
}

export function isPreviewable(file: { mime_type: string; file_name: string }): boolean {
  return pickViewer(file.mime_type, file.file_name) !== null;
}

export function FilePreviewModal({
  file,
  projectId,
  onClose,
}: {
  file: ProjectFile;
  projectId: number;
  onClose: () => void;
}) {
  const [buffer, setBuffer] = useState<ArrayBuffer | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const viewerType = pickViewer(file.mime_type, file.file_name);

  useEffect(() => {
    setBuffer(null);
    setLoading(true);
    setError(null);

    fetchProjectFileContent(projectId, file.id)
      .then(setBuffer)
      .catch((e) => setError(e.message || "Failed to load file"))
      .finally(() => setLoading(false));
  }, [projectId, file.id]);

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
      <div className="flex max-h-[85vh] w-full max-w-5xl flex-col overflow-hidden rounded-lg border border-white/[0.08] bg-[#0e0e10]">
        <div className="flex items-center justify-between border-b border-white/[0.06] px-4 py-3">
          <span className="truncate text-[12px] text-zinc-200">
            {file.file_name}
          </span>
          <button
            onClick={onClose}
            className="shrink-0 rounded p-1 text-zinc-500 hover:bg-white/[0.06] hover:text-zinc-300"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-auto">
          {loading && (
            <div className="flex h-48 items-center justify-center">
              <div className="h-5 w-5 animate-spin rounded-full border-2 border-zinc-600 border-t-zinc-300" />
            </div>
          )}
          {error && (
            <div className="flex h-48 flex-col items-center justify-center gap-2">
              <span className="text-[12px] text-red-400">{error}</span>
              <button
                onClick={() => {
                  setLoading(true);
                  setError(null);
                  fetchProjectFileContent(projectId, file.id)
                    .then(setBuffer)
                    .catch((e) => setError(e.message))
                    .finally(() => setLoading(false));
                }}
                className="rounded bg-white/[0.06] px-3 py-1 text-[11px] text-zinc-400 hover:bg-white/[0.1]"
              >
                Retry
              </button>
            </div>
          )}
          {!loading && !error && buffer && (
            <Suspense
              fallback={
                <div className="flex h-48 items-center justify-center">
                  <div className="h-5 w-5 animate-spin rounded-full border-2 border-zinc-600 border-t-zinc-300" />
                </div>
              }
            >
              {viewerType === "docx" && <DocxViewer buffer={buffer} />}
              {viewerType === "xlsx" && <XlsxViewer buffer={buffer} />}
              {viewerType === "pptx" && <PptxViewer buffer={buffer} />}
              {viewerType === "pdf" && <PdfViewer buffer={buffer} />}
              {!viewerType && (
                <div className="flex h-48 items-center justify-center text-[12px] text-zinc-500">
                  Preview not available for this file type.
                </div>
              )}
            </Suspense>
          )}
        </div>
      </div>
    </div>
  );
}
