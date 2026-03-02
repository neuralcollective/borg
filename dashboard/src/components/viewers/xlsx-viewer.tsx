import { useEffect, useState } from "react";

type SheetData = { name: string; rows: string[][] };

export function XlsxViewer({ buffer }: { buffer: ArrayBuffer }) {
  const [sheets, setSheets] = useState<SheetData[]>([]);
  const [active, setActive] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    import("xlsx").then((XLSX) => {
      try {
        const wb = XLSX.read(buffer, { type: "array" });
        const parsed: SheetData[] = wb.SheetNames.map((name) => {
          const ws = wb.Sheets[name];
          const rows: string[][] = XLSX.utils.sheet_to_json(ws, {
            header: 1,
            defval: "",
          });
          return { name, rows };
        });
        setSheets(parsed);
        setActive(0);
      } catch {
        setError("Failed to parse spreadsheet");
      }
    });
  }, [buffer]);

  if (error) return <div className="p-4 text-[12px] text-red-400">{error}</div>;
  if (!sheets.length) return null;

  const sheet = sheets[active];

  return (
    <div className="flex flex-col">
      {sheets.length > 1 && (
        <div className="flex gap-1 border-b border-white/[0.06] px-3 py-2">
          {sheets.map((s, i) => (
            <button
              key={s.name}
              onClick={() => setActive(i)}
              className={`rounded px-2 py-1 text-[11px] transition-colors ${
                i === active
                  ? "bg-white/[0.1] text-zinc-200"
                  : "text-zinc-500 hover:bg-white/[0.06] hover:text-zinc-400"
              }`}
            >
              {s.name}
            </button>
          ))}
        </div>
      )}
      <div className="overflow-auto">
        <table className="w-full border-collapse text-[11px]">
          <tbody>
            {sheet.rows.map((row, ri) => (
              <tr
                key={ri}
                className={ri === 0 ? "bg-white/[0.04]" : ""}
              >
                {row.map((cell, ci) => {
                  const Tag = ri === 0 ? "th" : "td";
                  return (
                    <Tag
                      key={ci}
                      className={`whitespace-nowrap border border-white/[0.06] px-2 py-1 text-left ${
                        ri === 0
                          ? "font-medium text-zinc-300"
                          : "text-zinc-400"
                      }`}
                    >
                      {cell}
                    </Tag>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
