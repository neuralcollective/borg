import { useEffect, useRef, useState } from "react";

export function PptxViewer({ buffer }: { buffer: ArrayBuffer }) {
  const ref = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!ref.current) return;
    const container = ref.current;
    container.innerHTML = "";

    import("pptx-preview").then((mod) => {
      try {
        const previewer = mod.init(container, {});
        previewer.preview(buffer);
      } catch {
        setError("Failed to render presentation");
      }
    });
  }, [buffer]);

  if (error) return <div className="p-4 text-[12px] text-red-400">{error}</div>;

  return (
    <div
      ref={ref}
      className="pptx-viewer flex flex-col items-center gap-4 overflow-auto p-4"
    />
  );
}
