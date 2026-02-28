import { useReducer, useMemo, useState, useCallback } from "react";
import { removeCustomMode, saveCustomMode, useCustomModes, useFullModes } from "@/lib/api";
import type { PipelineModeFull } from "@/lib/types";
import { cn } from "@/lib/utils";
import { ModeSidebar } from "./mode-creator/mode-sidebar";
import { ModeSettings } from "./mode-creator/mode-settings";
import { PhaseStrip } from "./mode-creator/phase-strip";
import { PhaseDetail } from "./mode-creator/phase-detail";
import { SeedList } from "./mode-creator/seed-list";
import { editorReducer, INITIAL_STATE, blankMode } from "./mode-creator/reducer";

const TABS = ["phases", "seeds", "json"] as const;

export function ModeCreatorPanel() {
  const { data: allModes = [], refetch: refetchAll } = useFullModes();
  const { data: customModes = [], refetch: refetchCustom } = useCustomModes();
  const [state, dispatch] = useReducer(editorReducer, INITIAL_STATE);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const customNameSet = useMemo(
    () => new Set(customModes.map((m) => m.name)),
    [customModes]
  );
  const builtInModes = useMemo(
    () => allModes.filter((m) => !customNameSet.has(m.name)),
    [allModes, customNameSet]
  );

  const handleSelect = useCallback((mode: PipelineModeFull, readOnly: boolean) => {
    dispatch({ type: "LOAD_MODE", mode, readOnly });
    setMsg("");
  }, []);

  const handleNew = useCallback(() => {
    dispatch({ type: "LOAD_MODE", mode: blankMode(), readOnly: false });
    setMsg("");
  }, []);

  const handleFork = useCallback(() => {
    const forkName = `${state.mode.name}_custom`;
    dispatch({ type: "FORK", newName: forkName });
    setMsg("");
  }, [state.mode.name]);

  const handleSave = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setMsg("");
    try {
      await saveCustomMode(state.mode);
      await Promise.all([refetchAll(), refetchCustom()]);
      dispatch({ type: "LOAD_MODE", mode: state.mode, readOnly: false });
      setMsg(`Saved '${state.mode.name}'`);
    } catch (err) {
      setMsg(`Save failed: ${err instanceof Error ? err.message : "unknown"}`);
    } finally {
      setBusy(false);
    }
  }, [busy, state.mode, refetchAll, refetchCustom]);

  const handleDiscard = useCallback(() => {
    const orig = JSON.parse(state.original) as PipelineModeFull;
    dispatch({ type: "LOAD_MODE", mode: orig, readOnly: state.isReadOnly });
    setMsg("");
  }, [state.original, state.isReadOnly]);

  const handleDelete = useCallback(async (name: string) => {
    if (busy) return;
    setBusy(true);
    setMsg("");
    try {
      await removeCustomMode(name);
      await Promise.all([refetchAll(), refetchCustom()]);
      if (state.mode.name === name) {
        dispatch({ type: "LOAD_MODE", mode: blankMode(), readOnly: false });
      }
      setMsg(`Deleted '${name}'`);
    } catch (err) {
      setMsg(`Delete failed: ${err instanceof Error ? err.message : "unknown"}`);
    } finally {
      setBusy(false);
    }
  }, [busy, state.mode.name, refetchAll, refetchCustom]);

  const { mode, selectedPhaseIndex, expandedSeedIndex, activeTab, isDirty, isReadOnly } = state;
  const selectedPhase = selectedPhaseIndex !== null ? mode.phases[selectedPhaseIndex] : null;
  const phaseNames = mode.phases.map((p) => p.name);

  return (
    <div className="flex h-full min-h-0">
      <ModeSidebar
        builtIn={builtInModes}
        custom={customModes}
        activeName={mode.name}
        onSelect={handleSelect}
        onNew={handleNew}
        onDelete={handleDelete}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        {/* Mode settings */}
        <div className="shrink-0 border-b border-white/[0.06] p-3">
          <ModeSettings
            mode={mode}
            readOnly={isReadOnly}
            onChange={(key, value) => dispatch({ type: "UPDATE_MODE", key, value })}
            onFork={handleFork}
          />
        </div>

        {/* Tab bar */}
        <div className="flex shrink-0 items-center gap-1 border-b border-white/[0.06] px-3 pt-1">
          {TABS.map((tab) => (
            <button
              key={tab}
              onClick={() => dispatch({ type: "SET_TAB", tab })}
              className={cn(
                "rounded-t-md px-3 py-1.5 text-[11px] font-medium capitalize transition-colors",
                activeTab === tab
                  ? "border border-b-0 border-white/[0.08] bg-white/[0.04] text-zinc-200"
                  : "text-zinc-500 hover:text-zinc-300"
              )}
            >
              {tab}
              {tab === "phases" && <span className="ml-1.5 text-zinc-600">{mode.phases.length}</span>}
              {tab === "seeds" && <span className="ml-1.5 text-zinc-600">{mode.seed_modes.length}</span>}
            </button>
          ))}
        </div>

        {/* Tab content */}
        <div className="flex-1 overflow-y-auto p-3">
          {activeTab === "phases" && (
            <div className="space-y-3">
              <PhaseStrip
                phases={mode.phases}
                selectedIndex={selectedPhaseIndex}
                readOnly={isReadOnly}
                onSelect={(i) => dispatch({ type: "SELECT_PHASE", index: i })}
                onAdd={(after) => dispatch({ type: "ADD_PHASE", afterIndex: after })}
                onRemove={(i) => dispatch({ type: "REMOVE_PHASE", index: i })}
                onMove={(from, to) => dispatch({ type: "MOVE_PHASE", from, to })}
              />
              {selectedPhase && selectedPhaseIndex !== null && (
                <PhaseDetail
                  phase={selectedPhase}
                  phaseNames={phaseNames}
                  readOnly={isReadOnly}
                  onChange={(patch) => dispatch({ type: "UPDATE_PHASE", index: selectedPhaseIndex, patch })}
                />
              )}
              {!selectedPhase && mode.phases.length > 0 && (
                <div className="rounded-lg border border-dashed border-white/[0.08] p-8 text-center text-[12px] text-zinc-600">
                  Select a phase above to edit its configuration
                </div>
              )}
            </div>
          )}

          {activeTab === "seeds" && (
            <SeedList
              seeds={mode.seed_modes}
              expandedIndex={expandedSeedIndex}
              readOnly={isReadOnly}
              onExpand={(i) => dispatch({ type: "EXPAND_SEED", index: i })}
              onUpdate={(i, patch) => dispatch({ type: "UPDATE_SEED", index: i, patch })}
              onAdd={() => dispatch({ type: "ADD_SEED" })}
              onRemove={(i) => dispatch({ type: "REMOVE_SEED", index: i })}
            />
          )}

          {activeTab === "json" && (
            <div className="relative">
              <pre className="min-h-[200px] rounded-lg border border-white/[0.06] bg-black/30 p-3 font-mono text-[11px] text-zinc-300 overflow-auto">
                {JSON.stringify(mode, null, 2)}
              </pre>
              <button
                onClick={() => navigator.clipboard.writeText(JSON.stringify(mode, null, 2))}
                className="absolute right-2 top-2 rounded-md bg-white/[0.06] px-2 py-1 text-[10px] text-zinc-500 hover:bg-white/[0.1] hover:text-zinc-300"
              >
                Copy
              </button>
            </div>
          )}
        </div>

        {/* Sticky save bar */}
        {(isDirty || msg) && (
          <div className="sticky bottom-0 flex shrink-0 items-center gap-2 border-t border-white/[0.08] bg-zinc-900/95 px-3 py-2 backdrop-blur">
            {isDirty && !isReadOnly && (
              <>
                <button
                  onClick={handleDiscard}
                  disabled={busy}
                  className="rounded-md bg-white/[0.06] px-3 py-1.5 text-[12px] text-zinc-400 hover:bg-white/[0.1] disabled:opacity-50"
                >
                  Discard
                </button>
                <button
                  onClick={handleSave}
                  disabled={busy || !mode.name.trim()}
                  className="rounded-md bg-blue-500/20 px-3 py-1.5 text-[12px] font-medium text-blue-400 ring-1 ring-inset ring-blue-500/20 hover:bg-blue-500/30 disabled:opacity-50"
                >
                  {busy ? "Saving..." : "Save"}
                </button>
              </>
            )}
            {msg && <span className="ml-auto text-[11px] text-zinc-500">{msg}</span>}
          </div>
        )}
      </div>
    </div>
  );
}
