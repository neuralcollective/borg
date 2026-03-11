import type { Change } from "diff";
import { diffLines, diffWords } from "diff";
import { useEffect, useMemo, useRef, useState } from "react";
import { apiBase, authHeaders } from "@/lib/api";
import { cn } from "@/lib/utils";

interface Version {
  sha: string;
  message: string;
  date: string;
  author: string;
}

interface RedlineViewerProps {
  projectId: number;
  taskId: number;
  path: string;
  versions: Version[];
}

type ViewMode = "inline" | "sidebyside";

interface LineDiff {
  type: "added" | "removed" | "unchanged";
  content: string;
  wordChanges?: Change[];
}

async function fetchDocumentContent(projectId: number, taskId: number, path: string, ref: string): Promise<string> {
  const url = `${apiBase()}/api/projects/${projectId}/documents/${taskId}/content?path=${encodeURIComponent(path)}&ref_name=${encodeURIComponent(ref)}`;
  const res = await fetch(url, { headers: authHeaders() });
  if (!res.ok) throw new Error(`Failed to fetch ${ref}: ${res.status}`);
  return res.text();
}

function computeLineDiffs(oldText: string, newText: string): LineDiff[] {
  const changes = diffLines(oldText, newText, { newlineIsToken: false });
  const result: LineDiff[] = [];

  for (const change of changes) {
    const lines = change.value.split("\n");
    // diffLines includes a trailing empty string when value ends with \n
    if (lines[lines.length - 1] === "") lines.pop();

    if (change.added) {
      for (const line of lines) result.push({ type: "added", content: line });
    } else if (change.removed) {
      for (const line of lines) result.push({ type: "removed", content: line });
    } else {
      for (const line of lines) result.push({ type: "unchanged", content: line });
    }
  }

  return result;
}

function computeSideBySide(lineDiffs: LineDiff[]): { left: LineDiff | null; right: LineDiff | null }[] {
  const rows: { left: LineDiff | null; right: LineDiff | null }[] = [];
  let i = 0;

  while (i < lineDiffs.length) {
    const curr = lineDiffs[i];
    if (curr.type === "unchanged") {
      rows.push({ left: curr, right: curr });
      i++;
    } else if (curr.type === "removed") {
      // Pair removals with following additions
      const nextAdded = lineDiffs[i + 1];
      if (nextAdded?.type === "added") {
        rows.push({ left: curr, right: nextAdded });
        i += 2;
      } else {
        rows.push({ left: curr, right: null });
        i++;
      }
    } else {
      // standalone addition
      rows.push({ left: null, right: curr });
      i++;
    }
  }

  return rows;
}

function getWordChanges(removed: string, added: string): { removedChanges: Change[]; addedChanges: Change[] } {
  const changes = diffWords(removed, added);
  const removedChanges: Change[] = [];
  const addedChanges: Change[] = [];

  for (const c of changes) {
    if (c.removed) removedChanges.push(c);
    else if (c.added) addedChanges.push(c);
    else {
      removedChanges.push(c);
      addedChanges.push(c);
    }
  }

  return { removedChanges, addedChanges };
}

function InlineLine({ line }: { line: LineDiff }) {
  if (line.type === "unchanged") {
    return (
      <div className="flex min-h-[1.4em]">
        <span className="w-6 shrink-0 select-none border-r border-white/[0.04] pr-1 text-right font-mono text-[10px] text-zinc-700" />
        <span className="flex-1 px-3 font-mono text-[11px] leading-relaxed text-zinc-400 whitespace-pre-wrap break-words">
          {line.content || " "}
        </span>
      </div>
    );
  }

  const isAdded = line.type === "added";
  const borderColor = isAdded ? "border-l-blue-600/60" : "border-l-red-600/60";
  const bgColor = isAdded ? "bg-blue-900/20" : "bg-red-900/20";
  const marker = isAdded ? "+" : "-";
  const markerColor = isAdded ? "text-blue-500" : "text-red-500";

  if (line.wordChanges && line.wordChanges.length > 0) {
    return (
      <div className={cn("flex min-h-[1.4em] border-l-2", borderColor, bgColor)}>
        <span
          className={cn(
            "w-6 shrink-0 select-none border-r border-white/[0.04] pr-1 text-right font-mono text-[10px]",
            markerColor,
          )}
        >
          {marker}
        </span>
        <span className="flex-1 px-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-words">
          {line.wordChanges.map((c, i) => {
            if (c.added) {
              return (
                <span key={i} className="rounded-sm bg-blue-700/40 text-blue-300 underline underline-offset-2">
                  {c.value}
                </span>
              );
            }
            if (c.removed) {
              return (
                <span key={i} className="rounded-sm bg-red-700/40 text-red-300 line-through">
                  {c.value}
                </span>
              );
            }
            return (
              <span key={i} className={isAdded ? "text-blue-200/80" : "text-red-200/80"}>
                {c.value}
              </span>
            );
          })}
        </span>
      </div>
    );
  }

  return (
    <div className={cn("flex min-h-[1.4em] border-l-2", borderColor, bgColor)}>
      <span
        className={cn(
          "w-6 shrink-0 select-none border-r border-white/[0.04] pr-1 text-right font-mono text-[10px]",
          markerColor,
        )}
      >
        {marker}
      </span>
      <span
        className={cn(
          "flex-1 px-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-words",
          isAdded ? "text-blue-300 underline underline-offset-2" : "text-red-300 line-through",
        )}
      >
        {line.content || " "}
      </span>
    </div>
  );
}

function SideCell({ line, side }: { line: LineDiff | null; side: "left" | "right" }) {
  if (!line) {
    return <div className="min-h-[1.4em] bg-white/[0.01]" />;
  }

  if (line.type === "unchanged") {
    return (
      <div className="min-h-[1.4em] px-3 font-mono text-[11px] leading-relaxed text-zinc-400 whitespace-pre-wrap break-words">
        {line.content || " "}
      </div>
    );
  }

  if (side === "left" && line.type === "removed") {
    const hasWordChanges = line.wordChanges && line.wordChanges.length > 0;
    return (
      <div className="min-h-[1.4em] border-l-2 border-l-red-600/60 bg-red-900/20 px-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-words">
        {hasWordChanges ? (
          line.wordChanges?.map((c, i) => {
            if (c.removed) {
              return (
                <span key={i} className="rounded-sm bg-red-700/40 text-red-300 line-through">
                  {c.value}
                </span>
              );
            }
            return (
              <span key={i} className="text-red-200/80">
                {c.value}
              </span>
            );
          })
        ) : (
          <span className="text-red-300 line-through">{line.content || " "}</span>
        )}
      </div>
    );
  }

  if (side === "right" && line.type === "added") {
    const hasWordChanges = line.wordChanges && line.wordChanges.length > 0;
    return (
      <div className="min-h-[1.4em] border-l-2 border-l-blue-600/60 bg-blue-900/20 px-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-words">
        {hasWordChanges ? (
          line.wordChanges?.map((c, i) => {
            if (c.added) {
              return (
                <span key={i} className="rounded-sm bg-blue-700/40 text-blue-300 underline underline-offset-2">
                  {c.value}
                </span>
              );
            }
            return (
              <span key={i} className="text-blue-200/80">
                {c.value}
              </span>
            );
          })
        ) : (
          <span className="text-blue-300 underline underline-offset-2">{line.content || " "}</span>
        )}
      </div>
    );
  }

  // Mismatched side/type (shouldn't happen in normal flow, render neutral)
  return (
    <div className="min-h-[1.4em] px-3 font-mono text-[11px] leading-relaxed text-zinc-500 whitespace-pre-wrap break-words">
      {line.content || " "}
    </div>
  );
}

function VersionSelect({
  label,
  versions,
  value,
  onChange,
}: {
  label: string;
  versions: Version[];
  value: string;
  onChange: (sha: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-[10px] font-medium uppercase tracking-wider text-zinc-600">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded-md border border-white/[0.08] bg-white/[0.04] px-2.5 py-1.5 text-[11px] text-zinc-300 outline-none focus:border-blue-500/40 min-w-[200px] max-w-[280px]"
      >
        {versions.map((v) => (
          <option key={v.sha} value={v.sha}>
            {v.sha.slice(0, 7)} — {v.message.length > 40 ? `${v.message.slice(0, 40)}…` : v.message}
          </option>
        ))}
      </select>
    </div>
  );
}

export function RedlineViewer({ projectId, taskId, path, versions }: RedlineViewerProps) {
  const defaultFrom = versions.length >= 2 ? versions[versions.length - 2].sha : (versions[0]?.sha ?? "");
  const defaultTo = versions.length >= 1 ? versions[versions.length - 1].sha : "";

  const [fromSha, setFromSha] = useState(defaultFrom);
  const [toSha, setToSha] = useState(defaultTo);
  const [viewMode, setViewMode] = useState<ViewMode>("inline");
  const [fromContent, setFromContent] = useState<string | null>(null);
  const [toContent, setToContent] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const leftScrollRef = useRef<HTMLDivElement>(null);
  const rightScrollRef = useRef<HTMLDivElement>(null);
  const syncingRef = useRef(false);

  useEffect(() => {
    if (!fromSha || !toSha) return;
    setLoadError(null);
    setLoading(true);
    setFromContent(null);
    setToContent(null);

    Promise.all([
      fetchDocumentContent(projectId, taskId, path, fromSha),
      fetchDocumentContent(projectId, taskId, path, toSha),
    ])
      .then(([from, to]) => {
        setFromContent(from);
        setToContent(to);
      })
      .catch((err: Error) => setLoadError(err.message))
      .finally(() => setLoading(false));
  }, [projectId, taskId, path, fromSha, toSha]);

  const lineDiffs = useMemo<LineDiff[]>(() => {
    if (fromContent === null || toContent === null) return [];

    const raw = computeLineDiffs(fromContent, toContent);

    // Pair adjacent removed/added lines to compute word-level diffs
    const result: LineDiff[] = [];
    let i = 0;
    while (i < raw.length) {
      const curr = raw[i];
      if (curr.type === "removed") {
        const next = raw[i + 1];
        if (next?.type === "added") {
          const { removedChanges, addedChanges } = getWordChanges(curr.content, next.content);
          result.push({ ...curr, wordChanges: removedChanges });
          result.push({ ...next, wordChanges: addedChanges });
          i += 2;
          continue;
        }
      }
      result.push(curr);
      i++;
    }
    return result;
  }, [fromContent, toContent]);

  const { additions, deletions } = useMemo(() => {
    let additions = 0;
    let deletions = 0;
    for (const line of lineDiffs) {
      if (line.type === "added") additions++;
      else if (line.type === "removed") deletions++;
    }
    return { additions, deletions };
  }, [lineDiffs]);

  const sideBySideRows = useMemo(
    () => (viewMode === "sidebyside" ? computeSideBySide(lineDiffs) : []),
    [viewMode, lineDiffs],
  );

  // Synchronized scrolling for side-by-side view
  function onLeftScroll() {
    if (syncingRef.current || !rightScrollRef.current || !leftScrollRef.current) return;
    syncingRef.current = true;
    rightScrollRef.current.scrollTop = leftScrollRef.current.scrollTop;
    syncingRef.current = false;
  }

  function onRightScroll() {
    if (syncingRef.current || !leftScrollRef.current || !rightScrollRef.current) return;
    syncingRef.current = true;
    leftScrollRef.current.scrollTop = rightScrollRef.current.scrollTop;
    syncingRef.current = false;
  }

  if (!versions.length) {
    return (
      <div className="flex h-full items-center justify-center text-[11px] text-zinc-600">No versions available</div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      {/* Controls bar */}
      <div className="flex shrink-0 flex-wrap items-end gap-4 border-b border-white/[0.06] px-4 py-3">
        <VersionSelect label="From Version" versions={versions} value={fromSha} onChange={setFromSha} />
        <VersionSelect label="To Version" versions={versions} value={toSha} onChange={setToSha} />

        <div className="ml-auto flex flex-col gap-1">
          <span className="text-[10px] font-medium uppercase tracking-wider text-zinc-600">View</span>
          <div className="flex rounded-md border border-white/[0.08]">
            <button
              onClick={() => setViewMode("inline")}
              className={cn(
                "px-3 py-1 text-[10px] font-medium transition-colors rounded-l-md",
                viewMode === "inline" ? "bg-white/[0.08] text-zinc-200" : "text-zinc-500 hover:text-zinc-300",
              )}
            >
              Inline
            </button>
            <button
              onClick={() => setViewMode("sidebyside")}
              className={cn(
                "border-l border-white/[0.08] px-3 py-1 text-[10px] font-medium transition-colors rounded-r-md",
                viewMode === "sidebyside" ? "bg-white/[0.08] text-zinc-200" : "text-zinc-500 hover:text-zinc-300",
              )}
            >
              Side by side
            </button>
          </div>
        </div>
      </div>

      {/* Summary bar */}
      {!loading && !loadError && lineDiffs.length > 0 && (
        <div className="flex shrink-0 items-center gap-4 border-b border-white/[0.04] bg-white/[0.01] px-4 py-1.5">
          <span className="text-[10px] text-zinc-500">
            <span className="font-mono font-semibold text-blue-400">+{additions}</span>
            <span className="ml-1">addition{additions !== 1 ? "s" : ""}</span>
          </span>
          <span className="text-[10px] text-zinc-500">
            <span className="font-mono font-semibold text-red-400">-{deletions}</span>
            <span className="ml-1">deletion{deletions !== 1 ? "s" : ""}</span>
          </span>
          {additions === 0 && deletions === 0 && <span className="text-[10px] text-zinc-600">No changes</span>}
        </div>
      )}

      {/* Content */}
      <div className="flex min-h-0 flex-1 overflow-hidden">
        {loading && <div className="flex flex-1 items-center justify-center text-[11px] text-zinc-600">Loading…</div>}

        {loadError && (
          <div className="flex flex-1 items-center justify-center px-4">
            <div className="rounded-lg border border-red-500/20 bg-red-500/[0.05] px-4 py-3 text-[11px] text-red-400">
              {loadError}
            </div>
          </div>
        )}

        {!loading && !loadError && lineDiffs.length === 0 && fromContent !== null && (
          <div className="flex flex-1 items-center justify-center text-[11px] text-zinc-600">
            No changes between selected versions
          </div>
        )}

        {!loading && !loadError && lineDiffs.length > 0 && viewMode === "inline" && (
          <div className="flex-1 overflow-y-auto overscroll-contain">
            <div className="divide-y divide-white/[0.02]">
              {lineDiffs.map((line, i) => (
                <InlineLine key={i} line={line} />
              ))}
            </div>
          </div>
        )}

        {!loading && !loadError && lineDiffs.length > 0 && viewMode === "sidebyside" && (
          <div className="flex min-h-0 flex-1">
            {/* Left column (old) */}
            <div
              ref={leftScrollRef}
              onScroll={onLeftScroll}
              className="flex-1 overflow-y-auto overscroll-contain border-r border-white/[0.06]"
            >
              <div className="sticky top-0 z-10 border-b border-white/[0.06] bg-zinc-950/90 px-3 py-1 backdrop-blur-sm">
                <span className="text-[9px] font-medium uppercase tracking-wider text-zinc-600">
                  {fromSha.slice(0, 7)}
                </span>
              </div>
              <div>
                {sideBySideRows.map((row, i) => (
                  <SideCell key={i} line={row.left} side="left" />
                ))}
              </div>
            </div>

            {/* Right column (new) */}
            <div ref={rightScrollRef} onScroll={onRightScroll} className="flex-1 overflow-y-auto overscroll-contain">
              <div className="sticky top-0 z-10 border-b border-white/[0.06] bg-zinc-950/90 px-3 py-1 backdrop-blur-sm">
                <span className="text-[9px] font-medium uppercase tracking-wider text-zinc-600">
                  {toSha.slice(0, 7)}
                </span>
              </div>
              <div>
                {sideBySideRows.map((row, i) => (
                  <SideCell key={i} line={row.right} side="right" />
                ))}
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
