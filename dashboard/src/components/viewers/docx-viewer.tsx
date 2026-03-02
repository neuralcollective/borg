import { useEffect, useRef, useState } from "react";

export function DocxViewer({ buffer }: { buffer: ArrayBuffer }) {
  const ref = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!ref.current) return;
    const container = ref.current;
    container.innerHTML = "";

    import("docx-preview").then(({ renderAsync }) => {
      renderAsync(buffer, container, undefined, {
        className: "docx-container",
        inWrapper: true,
        ignoreWidth: false,
        ignoreHeight: true,
      }).catch(() => setError("Failed to render document"));
    });
  }, [buffer]);

  if (error) return <div className="p-4 text-[12px] text-red-400">{error}</div>;

  return (
    <div
      ref={ref}
      className="docx-viewer overflow-auto bg-white p-4 [&_.docx-container]:mx-auto"
    />
  );
}
