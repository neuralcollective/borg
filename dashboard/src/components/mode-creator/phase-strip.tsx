import { useMemo } from "react";
import type { PhaseConfigFull, PhaseType } from "@/lib/types";
import { cn } from "@/lib/utils";

const TYPE_COLORS: Record<PhaseType, string> = {
  setup: "bg-zinc-700/50 text-zinc-400",
  agent: "bg-blue-500/15 text-blue-400",
  rebase: "bg-amber-500/15 text-amber-400",
  lint_fix: "bg-violet-500/15 text-violet-400",
};

const LOOP_COLORS = [
  "stroke-amber-500/50",
  "stroke-violet-500/50",
  "stroke-cyan-500/50",
  "stroke-rose-500/50",
];

const LOOP_FILL_COLORS = [
  "fill-amber-500/50",
  "fill-violet-500/50",
  "fill-cyan-500/50",
  "fill-rose-500/50",
];

const LOOP_TEXT_COLORS = [
  "fill-amber-500/60",
  "fill-violet-500/60",
  "fill-cyan-500/60",
  "fill-rose-500/60",
];

// Width of each phase column (node + connector) in px
const COL_W = 120;
const ARC_ROW_H = 28;

interface LoopEdge {
  fromIndex: number;
  toIndex: number;
  label: string;
}

export function PhaseStrip({
  phases,
  selectedIndex,
  readOnly,
  onSelect,
  onAdd,
  onRemove,
  onMove,
}: {
  phases: PhaseConfigFull[];
  selectedIndex: number | null;
  readOnly: boolean;
  onSelect: (index: number | null) => void;
  onAdd: (afterIndex: number) => void;
  onRemove: (index: number) => void;
  onMove: (from: number, to: number) => void;
}) {
  const nameToIndex = useMemo(() => {
    const map = new Map<string, number>();
    phases.forEach((p, i) => map.set(p.name, i));
    return map;
  }, [phases]);

  // Find backward edges (loops)
  const loops = useMemo(() => {
    const edges: LoopEdge[] = [];
    for (let i = 0; i < phases.length; i++) {
      const phase = phases[i];
      const targetIdx = nameToIndex.get(phase.next);
      // Backward edge: next points to an earlier or same phase
      if (targetIdx !== undefined && targetIdx <= i) {
        edges.push({ fromIndex: i, toIndex: targetIdx, label: phase.next });
      }
      // qa_fix routing creates an implicit loop
      if (phase.has_qa_fix_routing) {
        const qafIdx = nameToIndex.get("qa_fix");
        if (qafIdx !== undefined && qafIdx > i) {
          // Forward to qa_fix, but qa_fix loops back — already captured by qa_fix's next
        }
      }
    }
    return edges;
  }, [phases, nameToIndex]);

  // Stack overlapping loops at different heights
  const loopRows = useMemo(() => {
    const sorted = [...loops].sort((a, b) => (b.fromIndex - b.toIndex) - (a.fromIndex - a.toIndex));
    const rows: LoopEdge[][] = [];
    for (const edge of sorted) {
      let placed = false;
      for (const row of rows) {
        const overlaps = row.some(
          (e) =>
            Math.max(e.toIndex, edge.toIndex) < Math.min(e.fromIndex, edge.fromIndex) ||
            Math.max(e.fromIndex, edge.fromIndex) < Math.min(e.toIndex, edge.toIndex)
        );
        if (!overlaps) {
          row.push(edge);
          placed = true;
          break;
        }
      }
      if (!placed) rows.push([edge]);
    }
    return rows;
  }, [loops]);

  const totalW = (phases.length + 1) * COL_W;
  const arcH = loopRows.length * ARC_ROW_H + (loopRows.length > 0 ? 8 : 0);

  return (
    <div className="space-y-2">
      <div className="overflow-x-auto pb-1">
        <div style={{ minWidth: totalW }}>
          {/* Phase nodes row */}
          <div className="flex items-center">
            {phases.map((phase, i) => {
              const selected = i === selectedIndex;
              return (
                <div key={`${phase.name}-${i}`} className="flex shrink-0 items-center" style={{ width: COL_W }}>
                  {i > 0 && (
                    <div className="flex items-center">
                      <div className="h-px w-3 bg-zinc-700" />
                      <span className="text-[9px] text-zinc-700">&rsaquo;</span>
                      <div className="h-px w-3 bg-zinc-700" />
                    </div>
                  )}
                  <button
                    onClick={() => onSelect(selected ? null : i)}
                    className={cn(
                      "flex-1 rounded-lg border px-3 py-2 text-left transition-colors",
                      selected
                        ? "border-blue-500/30 bg-blue-500/[0.06] ring-1 ring-blue-500/40"
                        : "border-white/[0.08] bg-white/[0.03] hover:bg-white/[0.06]"
                    )}
                  >
                    <div className={cn(
                      "text-[11px] font-medium truncate",
                      selected ? "text-zinc-100" : "text-zinc-300"
                    )}>
                      {phase.name}
                    </div>
                    <span className={cn("mt-0.5 inline-block rounded px-1 py-px text-[9px]", TYPE_COLORS[phase.phase_type])}>
                      {phase.phase_type}
                    </span>
                  </button>
                </div>
              );
            })}

            {/* Terminal "done" node */}
            <div className="flex shrink-0 items-center" style={{ width: COL_W }}>
              <div className="flex items-center">
                <div className="h-px w-3 bg-zinc-700" />
                <span className="text-[9px] text-zinc-700">&rsaquo;</span>
                <div className="h-px w-3 bg-zinc-700" />
              </div>
              <div className="flex-1 rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2">
                <div className="text-[11px] font-medium text-zinc-600">done</div>
              </div>
            </div>
          </div>

          {/* Loop arcs */}
          {loopRows.length > 0 && (
            <svg width={totalW} height={arcH} className="mt-1">
              {loopRows.map((row, rowIdx) =>
                row.map((edge, edgeIdx) => {
                  const colorIdx = (rowIdx + edgeIdx) % LOOP_COLORS.length;
                  // Center of source and target nodes
                  const fromX = edge.fromIndex * COL_W + COL_W / 2;
                  const toX = edge.toIndex * COL_W + COL_W / 2;
                  const y0 = 4;
                  const y1 = (rowIdx + 1) * ARC_ROW_H;
                  const midX = (fromX + toX) / 2;

                  // Curved path: down from source, arc to target, up to target
                  const d = `M ${fromX} ${y0} L ${fromX} ${y1} Q ${fromX} ${y1 + 8} ${fromX - 8} ${y1 + 8} L ${toX + 8} ${y1 + 8} Q ${toX} ${y1 + 8} ${toX} ${y1} L ${toX} ${y0}`;

                  return (
                    <g key={`${edge.fromIndex}-${edge.toIndex}`}>
                      <path
                        d={d}
                        className={LOOP_COLORS[colorIdx]}
                        fill="none"
                        strokeWidth={1.5}
                        strokeDasharray="4 2"
                      />
                      {/* Arrow at target */}
                      <polygon
                        points={`${toX - 3},${y0 + 5} ${toX + 3},${y0 + 5} ${toX},${y0}`}
                        className={LOOP_FILL_COLORS[colorIdx]}
                      />
                      {/* Label */}
                      <text
                        x={midX}
                        y={y1 + 5}
                        textAnchor="middle"
                        className={cn("text-[8px]", LOOP_TEXT_COLORS[colorIdx])}
                      >
                        loop
                      </text>
                    </g>
                  );
                })
              )}
            </svg>
          )}
        </div>
      </div>

      {/* Actions bar */}
      {!readOnly && (
        <div className="flex items-center gap-2">
          <button
            onClick={() => onAdd(selectedIndex ?? phases.length - 1)}
            className="rounded-md bg-white/[0.06] px-2 py-1 text-[11px] text-zinc-400 hover:bg-white/[0.1]"
          >
            + Add Phase
          </button>
          {selectedIndex !== null && (
            <>
              <button
                onClick={() => { if (selectedIndex > 0) onMove(selectedIndex, selectedIndex - 1); }}
                disabled={selectedIndex <= 0}
                className="rounded-md bg-white/[0.06] px-2 py-1 text-[11px] text-zinc-400 hover:bg-white/[0.1] disabled:opacity-30"
              >
                &larr;
              </button>
              <button
                onClick={() => { if (selectedIndex < phases.length - 1) onMove(selectedIndex, selectedIndex + 1); }}
                disabled={selectedIndex >= phases.length - 1}
                className="rounded-md bg-white/[0.06] px-2 py-1 text-[11px] text-zinc-400 hover:bg-white/[0.1] disabled:opacity-30"
              >
                &rarr;
              </button>
              <button
                onClick={() => onRemove(selectedIndex)}
                className="rounded-md bg-red-500/10 px-2 py-1 text-[11px] text-red-400 hover:bg-red-500/20"
              >
                Remove
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
