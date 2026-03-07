import { useEffect, useRef, useState } from "react";

export function PdfViewer({ buffer }: { buffer: ArrayBuffer }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;
    const container = containerRef.current;
    container.innerHTML = "";

    import("pdfjs-dist").then(async (pdfjs) => {
      pdfjs.GlobalWorkerOptions.workerSrc = new URL(
        "pdfjs-dist/build/pdf.worker.min.mjs",
        import.meta.url
      ).toString();

      try {
        const doc = await pdfjs.getDocument({ data: buffer.slice(0) }).promise;

        for (let i = 1; i <= doc.numPages; i++) {
          const page = await doc.getPage(i);
          const scale = 1.5;
          const viewport = page.getViewport({ scale });

          const canvas = document.createElement("canvas");
          canvas.width = viewport.width;
          canvas.height = viewport.height;
          canvas.className = "mx-auto shadow-lg";

          const ctx = canvas.getContext("2d");
          if (!ctx) continue;
          await page.render({ canvasContext: ctx, viewport } as any).promise;
          container.appendChild(canvas);
        }
      } catch {
        setError("Failed to render PDF");
      }
    });
  }, [buffer]);

  if (error) return <div className="p-4 text-[12px] text-red-400">{error}</div>;

  return (
    <div
      ref={containerRef}
      className="flex flex-col items-center gap-4 overflow-auto bg-zinc-800 p-4"
    />
  );
}
