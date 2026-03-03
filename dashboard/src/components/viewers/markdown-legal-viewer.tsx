import { useEffect, useMemo, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { PrismLight as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { apiBase, authHeaders, tokenReady } from "@/lib/api";

// ── Types ─────────────────────────────────────────────────────────────────────

interface DocumentVersion {
  sha: string;
  message: string;
  date: string;
  author: string;
}

type CitationStatus = "verified" | "unverified" | "flagged";

interface Citation {
  text: string;
  status: CitationStatus;
  anchorId: string;
  index: number;
}

export interface MarkdownLegalViewerProps {
  projectId: number;
  taskId: number;
  path: string;
}

// ── Citation parsing ──────────────────────────────────────────────────────────

const CITATION_PATTERNS = [
  // Case citations: Smith v. Jones, 123 U.S. 456 (1999)
  /[A-Z][A-Za-z\s'&,.-]+\s+v\.\s+[A-Z][A-Za-z\s'&,.-]+,\s*\d+\s+[A-Z][A-Z.]+\d*\s+\d+\s*\(\d{4}\)/g,
  // US Code: 42 U.S.C. § 1983
  /\d+\s+U\.S\.C\.\s+§+\s*\d+[\w-]*/g,
  // CFR citations: 29 C.F.R. § 541.100
  /\d+\s+C\.F\.R\.\s+§+\s*\d+[\d.]*/g,
  // UK Supreme Court: [2023] UKSC 12
  /\[\d{4}\]\s+(?:UKSC|EWCA|EWHC|UKHL|UKPC|EWCOP)\s+\d+/g,
  // Federal Reporter: 123 F.3d 456 (9th Cir. 2001)
  /\d+\s+F\.(?:\d+d|Supp\.(?:\s*\d+d)?)\s+\d+\s*\([^)]+\d{4}\)/g,
  // Supreme Court Reporter: 123 S. Ct. 456
  /\d+\s+S\.\s*Ct\.\s+\d+/g,
  // Statute sections: § 1983
  /§+\s*\d+[\w.-]*/g,
];

function extractCitations(content: string): Citation[] {
  const found: Array<{ text: string; index: number }> = [];
  const seen = new Set<string>();

  for (const pattern of CITATION_PATTERNS) {
    const re = new RegExp(pattern.source, pattern.flags);
    let m: RegExpExecArray | null;
    while ((m = re.exec(content)) !== null) {
      const text = m[0].trim();
      if (!seen.has(text)) {
        seen.add(text);
        found.push({ text, index: m.index });
      }
    }
  }

  found.sort((a, b) => a.index - b.index);

  return found.map((f, i) => ({
    text: f.text,
    status: deriveCitationStatus(content, f.index, f.text),
    anchorId: `citation-${i}`,
    index: i,
  }));
}

function deriveCitationStatus(content: string, pos: number, _text: string): CitationStatus {
  const window = content.slice(Math.max(0, pos - 200), pos + 200).toUpperCase();
  if (window.includes("FLAGGED") || window.includes("INVALID") || window.includes("OVERRULED")) {
    return "flagged";
  }
  if (window.includes("VERIFIED") || window.includes("CONFIRMED")) {
    return "verified";
  }
  if (window.includes("UNVERIFIED") || window.includes("TRAINING-DATA-ONLY") || window.includes("TRAINING DATA")) {
    return "unverified";
  }
  return "unverified";
}

// ── Markdown components ───────────────────────────────────────────────────────

function CodeBlock({
  className,
  children,
  ...props
}: React.HTMLAttributes<HTMLElement> & { children?: React.ReactNode }) {
  const match = /language-(\w+)/.exec(className || "");
  const code = String(children).replace(/\n$/, "");

  if (!match) {
    return (
      <code className="rounded bg-white/[0.08] px-1 text-[12px] text-orange-300" {...props}>
        {children}
      </code>
    );
  }

  return (
    <SyntaxHighlighter
      style={oneDark}
      language={match[1]}
      PreTag="div"
      customStyle={{
        margin: "0.5rem 0",
        padding: "0.75rem",
        borderRadius: "0.375rem",
        fontSize: "12px",
        lineHeight: "1.6",
        background: "rgba(255,255,255,0.04)",
      }}
      codeTagProps={{ style: { fontFamily: "inherit" } }}
    >
      {code}
    </SyntaxHighlighter>
  );
}

function ConfidenceInline({ level }: { level: string }) {
  const lc = level.toLowerCase();
  return (
    <span
      className={cn(
        "ml-1 inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
        lc === "high" && "bg-emerald-500/15 text-emerald-400",
        lc === "medium" && "bg-amber-500/15 text-amber-400",
        lc === "low" && "bg-red-500/15 text-red-400"
      )}
    >
      {level}
    </span>
  );
}

const remarkPlugins = [remarkGfm];

// ── Status dot ────────────────────────────────────────────────────────────────

function StatusDot({ status }: { status: CitationStatus }) {
  return (
    <span
      className={cn(
        "inline-block h-2 w-2 shrink-0 rounded-full",
        status === "verified" && "bg-emerald-400",
        status === "unverified" && "bg-amber-400",
        status === "flagged" && "bg-red-400"
      )}
      title={status}
    />
  );
}

// ── API helpers ───────────────────────────────────────────────────────────────

async function fetchDocumentContent(
  projectId: number,
  taskId: number,
  path: string,
  ref?: string
): Promise<string> {
  await tokenReady;
  let url = `${apiBase()}/api/projects/${projectId}/documents/${taskId}/content?path=${encodeURIComponent(path)}`;
  if (ref) url += `&ref_name=${encodeURIComponent(ref)}`;
  const res = await fetch(url, { headers: authHeaders() });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.text();
}

async function fetchDocumentVersions(
  projectId: number,
  taskId: number,
  path: string
): Promise<DocumentVersion[]> {
  await tokenReady;
  const url = `${apiBase()}/api/projects/${projectId}/documents/${taskId}/versions?path=${encodeURIComponent(path)}`;
  const res = await fetch(url, { headers: authHeaders() });
  if (!res.ok) return [];
  return res.json();
}

// ── Main component ────────────────────────────────────────────────────────────

export function MarkdownLegalViewer({ projectId, taskId, path }: MarkdownLegalViewerProps) {
  const [content, setContent] = useState<string>("");
  const [versions, setVersions] = useState<DocumentVersion[]>([]);
  const [selectedSha, setSelectedSha] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeCitation, setActiveCitation] = useState<string | null>(null);
  const [exportOpen, setExportOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const exportRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!exportOpen) return;
    function handleClick(e: MouseEvent) {
      if (exportRef.current && !exportRef.current.contains(e.target as Node)) {
        setExportOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [exportOpen]);

  async function triggerExport(format: "pdf" | "docx") {
    setExportOpen(false);
    setExporting(true);
    try {
      await tokenReady;
      let url = `${apiBase()}/api/projects/${projectId}/documents/${taskId}/export?path=${encodeURIComponent(path)}&format=${format}`;
      if (selectedSha) url += `&ref_name=${encodeURIComponent(selectedSha)}`;
      const res = await fetch(url, { headers: authHeaders() });
      if (!res.ok) {
        const text = await res.text();
        alert(`Export failed: ${text || res.status}`);
        return;
      }
      const blob = await res.blob();
      const blobUrl = URL.createObjectURL(blob);
      const a = document.createElement("a");
      const stem = path.split("/").pop()?.replace(/\.\w+$/, "") ?? "document";
      a.href = blobUrl;
      a.download = `${stem}.${format}`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(blobUrl);
    } finally {
      setExporting(false);
    }
  }

  useEffect(() => {
    fetchDocumentVersions(projectId, taskId, path)
      .then(setVersions)
      .catch(() => setVersions([]));
  }, [projectId, taskId, path]);

  useEffect(() => {
    setLoading(true);
    setError(null);
    const ref = selectedSha || undefined;
    fetchDocumentContent(projectId, taskId, path, ref)
      .then((text) => {
        setContent(text);
        setLoading(false);
      })
      .catch((e) => {
        setError(e.message || "Failed to load document");
        setLoading(false);
      });
  }, [projectId, taskId, path, selectedSha]);

  const isPrivileged = useMemo(
    () => /PRIVILEGED\s+AND\s+CONFIDENTIAL/i.test(content),
    [content]
  );

  const citations = useMemo(() => extractCitations(content), [content]);

  function scrollToCitation(citation: Citation) {
    setActiveCitation(citation.anchorId);
    if (!contentRef.current) return;

    const allText = contentRef.current.querySelectorAll("p, li, blockquote, td");
    for (const el of allText) {
      if (el.textContent?.includes(citation.text.slice(0, 30))) {
        el.scrollIntoView({ behavior: "smooth", block: "center" });
        (el as HTMLElement).classList.add("citation-highlight");
        setTimeout(() => (el as HTMLElement).classList.remove("citation-highlight"), 2000);
        break;
      }
    }
  }

  const mdComponents = useMemo(
    () => ({
      code: CodeBlock as any,
      p: ({ children }: { children?: React.ReactNode }) => (
        <ParagraphNode citations={citations}>{children}</ParagraphNode>
      ),
      blockquote: ({ children }: { children?: React.ReactNode }) => (
        <blockquote className="my-3 border-l-2 border-blue-500/40 bg-blue-500/[0.04] py-2 pl-4 text-[13px] italic text-zinc-400">
          {children}
        </blockquote>
      ),
      h1: ({ children }: { children?: React.ReactNode }) => (
        <h1 className="mb-3 mt-6 border-b border-white/[0.08] pb-2 text-[18px] font-semibold text-zinc-100">
          {children}
        </h1>
      ),
      h2: ({ children }: { children?: React.ReactNode }) => (
        <h2 className="mb-2 mt-5 text-[15px] font-semibold text-zinc-200">{children}</h2>
      ),
      h3: ({ children }: { children?: React.ReactNode }) => (
        <h3 className="mb-2 mt-4 text-[13px] font-semibold text-zinc-300">{children}</h3>
      ),
      table: ({ children }: { children?: React.ReactNode }) => (
        <div className="my-3 overflow-x-auto rounded border border-white/[0.08]">
          <table className="w-full text-[12px]">{children}</table>
        </div>
      ),
      th: ({ children }: { children?: React.ReactNode }) => (
        <th className="border-b border-white/[0.1] bg-white/[0.04] px-3 py-2 text-left font-medium text-zinc-400">
          {children}
        </th>
      ),
      td: ({ children }: { children?: React.ReactNode }) => (
        <td className="border-b border-white/[0.05] px-3 py-2 text-zinc-300">{children}</td>
      ),
      strong: ({ children }: { children?: React.ReactNode }) => (
        <strong className="font-semibold text-zinc-200">{children}</strong>
      ),
      a: ({ href, children }: { href?: string; children?: React.ReactNode }) => (
        <a
          href={href}
          target="_blank"
          rel="noopener noreferrer"
          className="text-blue-400 underline underline-offset-2 hover:text-blue-300"
        >
          {children}
        </a>
      ),
      ul: ({ children }: { children?: React.ReactNode }) => (
        <ul className="my-2 space-y-1 pl-5 text-[13px] text-zinc-300 [&_li]:list-disc">{children}</ul>
      ),
      ol: ({ children }: { children?: React.ReactNode }) => (
        <ol className="my-2 space-y-1 pl-5 text-[13px] text-zinc-300 [&_li]:list-decimal">{children}</ol>
      ),
    }),
    [citations]
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* Header bar */}
      <div className="flex shrink-0 items-center gap-3 border-b border-white/[0.06] px-4 py-2.5">
        <span className="truncate text-[12px] font-medium text-zinc-300">{path}</span>
        {versions.length > 0 && (
          <select
            value={selectedSha}
            onChange={(e) => setSelectedSha(e.target.value)}
            className="ml-auto shrink-0 rounded border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-[11px] text-zinc-400 outline-none focus:border-blue-500/40"
          >
            <option value="">Latest</option>
            {versions.map((v) => (
              <option key={v.sha} value={v.sha}>
                {v.sha.slice(0, 7)} — {v.message.slice(0, 40)}{v.message.length > 40 ? "…" : ""} ({v.date.slice(0, 10)})
              </option>
            ))}
          </select>
        )}
        {/* Export dropdown */}
        <div ref={exportRef} className={cn("relative shrink-0", versions.length === 0 && "ml-auto")}>
          <button
            onClick={() => setExportOpen((v) => !v)}
            disabled={exporting || loading}
            className="flex items-center gap-1.5 rounded border border-white/[0.08] bg-white/[0.04] px-2.5 py-1 text-[11px] text-zinc-400 transition-colors hover:border-white/[0.14] hover:text-zinc-300 disabled:opacity-50"
          >
            {exporting ? (
              <span className="h-3 w-3 animate-spin rounded-full border border-zinc-500 border-t-zinc-300" />
            ) : (
              <svg className="h-3 w-3" fill="none" viewBox="0 0 16 16" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 8v5h8V8M8 2v8M5.5 7.5 8 10l2.5-2.5" />
              </svg>
            )}
            Export
            <svg className="h-2.5 w-2.5 opacity-60" fill="none" viewBox="0 0 10 10" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" d="M2 3.5 5 6.5 8 3.5" />
            </svg>
          </button>
          {exportOpen && (
            <div className="absolute right-0 top-full z-50 mt-1 w-40 overflow-hidden rounded border border-white/[0.1] bg-zinc-900 shadow-xl">
              <button
                onClick={() => triggerExport("pdf")}
                className="flex w-full items-center gap-2 px-3 py-2 text-left text-[12px] text-zinc-300 transition-colors hover:bg-white/[0.06]"
              >
                Export as PDF
              </button>
              <button
                onClick={() => triggerExport("docx")}
                className="flex w-full items-center gap-2 px-3 py-2 text-left text-[12px] text-zinc-300 transition-colors hover:bg-white/[0.06]"
              >
                Export as DOCX
              </button>
            </div>
          )}
        </div>
      </div>

      {/* Privilege banner */}
      {isPrivileged && (
        <div className="shrink-0 bg-red-900/40 px-4 py-2 text-center text-[11px] font-semibold uppercase tracking-widest text-red-300 ring-1 ring-inset ring-red-500/30">
          Privileged and Confidential — Attorney-Client Communication
        </div>
      )}

      {/* Body: document + sidebar */}
      <div className="flex min-h-0 flex-1">
        {/* Document content */}
        <ScrollArea className="min-w-0 flex-1">
          <div className="px-6 py-5">
            {loading && (
              <div className="flex h-40 items-center justify-center">
                <div className="h-5 w-5 animate-spin rounded-full border-2 border-zinc-600 border-t-zinc-300" />
              </div>
            )}
            {error && (
              <div className="rounded border border-red-500/20 bg-red-500/[0.05] p-4 text-[12px] text-red-400">
                {error}
              </div>
            )}
            {!loading && !error && (
              <div ref={contentRef} className="[&_.citation-highlight]:bg-amber-500/20 [&_.citation-highlight]:transition-colors">
                <ConfidenceAwareMarkdown content={content} components={mdComponents} />
              </div>
            )}
          </div>
        </ScrollArea>

        {/* Citation sidebar */}
        {citations.length > 0 && (
          <div className="flex w-[280px] shrink-0 flex-col border-l border-white/[0.06]">
            <div className="shrink-0 border-b border-white/[0.06] px-3 py-2.5">
              <div className="text-[11px] font-medium uppercase tracking-wide text-zinc-500">
                Citations ({citations.length})
              </div>
              <div className="mt-1.5 flex items-center gap-3 text-[10px] text-zinc-600">
                <span className="flex items-center gap-1">
                  <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
                  verified
                </span>
                <span className="flex items-center gap-1">
                  <span className="h-1.5 w-1.5 rounded-full bg-amber-400" />
                  unverified
                </span>
                <span className="flex items-center gap-1">
                  <span className="h-1.5 w-1.5 rounded-full bg-red-400" />
                  flagged
                </span>
              </div>
            </div>
            <ScrollArea className="flex-1">
              <div className="space-y-px p-2">
                {citations.map((c) => (
                  <button
                    key={c.anchorId}
                    onClick={() => scrollToCitation(c)}
                    className={cn(
                      "flex w-full items-start gap-2.5 rounded px-2 py-2 text-left transition-colors hover:bg-white/[0.05]",
                      activeCitation === c.anchorId && "bg-white/[0.07]"
                    )}
                  >
                    <StatusDot status={c.status} />
                    <span className="text-[11px] leading-snug text-zinc-400">{c.text}</span>
                  </button>
                ))}
              </div>
            </ScrollArea>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Confidence-aware renderer ─────────────────────────────────────────────────

function ParagraphNode({
  children,
  citations: _citations,
}: {
  children?: React.ReactNode;
  citations: Citation[];
}) {
  const processed = processConfidenceInChildren(children);
  return <p className="my-2 text-[13px] leading-relaxed text-zinc-300">{processed}</p>;
}

function processConfidenceInChildren(children: React.ReactNode): React.ReactNode {
  if (typeof children === "string") {
    return splitConfidence(children);
  }
  if (Array.isArray(children)) {
    return children.map((child, i) => (
      <span key={i}>{processConfidenceInChildren(child)}</span>
    ));
  }
  return children;
}

function splitConfidence(text: string): React.ReactNode {
  const parts = text.split(/(\bConfidence:\s*(?:High|Medium|Low)\b)/gi);
  if (parts.length === 1) return text;
  return parts.map((part, i) => {
    const m = /^Confidence:\s*(High|Medium|Low)$/i.exec(part);
    if (m) {
      return (
        <span key={i} className="inline-flex items-center gap-1">
          <span className="text-zinc-500">Confidence:</span>
          <ConfidenceInline level={m[1]} />
        </span>
      );
    }
    return <span key={i}>{part}</span>;
  });
}

function ConfidenceAwareMarkdown({
  content,
  components,
}: {
  content: string;
  components: Record<string, unknown>;
}) {
  return (
    <Markdown remarkPlugins={remarkPlugins} components={components as any}>
      {content}
    </Markdown>
  );
}
