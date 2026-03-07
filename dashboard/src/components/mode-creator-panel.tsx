import { useReducer, useMemo, useState, useCallback } from "react";
import { removeCustomMode, saveCustomMode, useCustomModes, useFullModes, useSettings } from "@/lib/api";
import type { PipelineModeFull } from "@/lib/types";
import { cn } from "@/lib/utils";
import { ModeSidebar } from "./mode-creator/mode-sidebar";
import { ModeSettings } from "./mode-creator/mode-settings";
import { PhaseStrip } from "./mode-creator/phase-strip";
import { PhaseDetail } from "./mode-creator/phase-detail";
import { SeedList } from "./mode-creator/seed-list";
import { editorReducer, INITIAL_STATE, blankMode } from "./mode-creator/reducer";
import { getProfile } from "./mode-creator/category-profiles";
import { useDashboardMode } from "@/lib/dashboard-mode";
import { Layers } from "lucide-react";

const TABS = ["phases", "seeds", "json"] as const;
const CORE_MODES = new Set(["sweborg", "lawborg", "swe", "legal", "knowledge"]);

export function ModeCreatorPanel() {
  const { data: allModes = [], refetch: refetchAll } = useFullModes();
  const { data: customModes = [], refetch: refetchCustom } = useCustomModes();
  const { data: settings } = useSettings();
  const [state, dispatch] = useReducer(editorReducer, INITIAL_STATE);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [showAllOptions, setShowAllOptions] = useState(false);

  const { isSWE, mode: dashboardMode } = useDashboardMode();
  const allowExperimental = settings?.experimental_domains === true;
  const visibleCats = useMemo(() => {
    const raw = settings?.visible_categories ?? "";
    const cats = raw.split(",").map((s) => s.trim()).filter(Boolean);
    return cats.length > 0 ? new Set(cats) : null;
  }, [settings?.visible_categories]);

  const customNameSet = useMemo(
    () => new Set(customModes.map((m) => m.name)),
    [customModes]
  );
  const builtInModes = useMemo(
    () =>
      allModes.filter(
        (m) =>
          !customNameSet.has(m.name) &&
          (allowExperimental || CORE_MODES.has(m.name)) &&
          (visibleCats === null || visibleCats.has(m.category || ""))
      ),
    [allModes, customNameSet, allowExperimental, visibleCats]
  );

  const handleSelect = useCallback((mode: PipelineModeFull, readOnly: boolean) => {
    dispatch({ type: "LOAD_MODE", mode, readOnly });
    setMsg("");
  }, []);

  const handleNew = useCallback(() => {
    const mode = blankMode();
    if (isSWE) {
      if (!allowExperimental && (mode.category ?? "").toLowerCase() !== "engineering") {
        mode.category = "Engineering";
      }
    } else {
      mode.category = "Professional Services";
      mode.integration = "none" as PipelineModeFull["integration"];
    }
    dispatch({ type: "LOAD_MODE", mode, readOnly: false });
    setMsg("");
  }, [allowExperimental, isSWE]);

  const handleFork = useCallback(() => {
    const forkName = `${state.mode.name}_custom`;
    dispatch({ type: "FORK", newName: forkName });
    setMsg("");
  }, [state.mode.name]);

  const handleSave = useCallback(async () => {
    if (busy) return;
    if (!allowExperimental && !CORE_MODES.has(state.mode.name)) {
      setMsg("Save blocked: enable Experimental Domains in Settings for non-core mode names.");
      return;
    }
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
  }, [allowExperimental, busy, state.mode, refetchAll, refetchCustom]);

  const handleDiscard = useCallback(() => {
    if (!state.original) return;
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
  const profile = useMemo(() => getProfile(mode.category || "", showAllOptions, dashboardMode), [mode.category, showAllOptions, dashboardMode]);

  return (
    <div className="flex h-full min-h-0">
      <ModeSidebar
        builtIn={builtInModes}
        custom={customModes}
        allowExperimental={allowExperimental}
        activeName={mode.name}
        onSelect={handleSelect}
        onNew={handleNew}
        onDelete={handleDelete}
      />

      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        {/* Fork banner for built-in modes */}
        {isReadOnly && mode.name && (
          <button
            onClick={handleFork}
            className="flex shrink-0 items-center justify-between border-b border-amber-500/20 bg-amber-500/[0.04] px-5 py-3 text-left transition-colors hover:bg-amber-500/[0.08]"
          >
            <div>
              <div className="text-[13px] font-medium text-amber-300">
                Viewing built-in template
              </div>
              <div className="text-[12px] text-amber-400/50">
                Click to create an editable copy
              </div>
            </div>
            <span className="rounded-lg bg-amber-500/15 px-4 py-2 text-[13px] font-medium text-amber-300 ring-1 ring-inset ring-amber-500/20">
              Fork &amp; Customize
            </span>
          </button>
        )}

        {/* Header + Mode settings */}
        <div className="shrink-0 border-b border-[#2a2520] p-5">
          {!mode.name && (
            <div className="flex items-center gap-3 mb-5">
              <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-[#1c1a17] ring-1 ring-amber-900/20">
                <Layers className="h-6 w-6 text-amber-400/60" />
              </div>
              <div>
                <h2 className="text-[20px] font-semibold text-[#e8e0d4]">Pipeline Creator</h2>
                <p className="text-[13px] text-[#6b6459]">Design and customize pipeline modes for your agents.</p>
              </div>
            </div>
          )}
          <ModeSettings
            mode={mode}
            readOnly={isReadOnly}
            onChange={(key, value) => dispatch({ type: "UPDATE_MODE", key, value })}
            profile={profile}
          />
        </div>

        {/* Tab bar */}
        <div className="flex shrink-0 items-center gap-1.5 border-b border-[#2a2520] px-5 pt-1.5">
          {TABS.map((tab) => (
            <button
              key={tab}
              onClick={() => dispatch({ type: "SET_TAB", tab })}
              className={cn(
                "rounded-t-lg px-4 py-2 text-[13px] font-medium capitalize transition-colors",
                activeTab === tab
                  ? "border-b-2 border-amber-400 text-[#e8e0d4]"
                  : "text-[#6b6459] hover:text-[#9c9486]"
              )}
            >
              {tab}
              {tab === "phases" && <span className="ml-2 text-[#6b6459]">{mode.phases.length}</span>}
              {tab === "seeds" && <span className="ml-2 text-[#6b6459]">{mode.seed_modes.length}</span>}
            </button>
          ))}
          <div className="ml-auto flex items-center gap-2">
            <button
              onClick={() => setShowAllOptions(!showAllOptions)}
              className={cn(
                "rounded-lg px-3 py-1.5 text-[12px] transition-colors",
                showAllOptions
                  ? "bg-amber-500/15 text-amber-300 ring-1 ring-inset ring-amber-500/20"
                  : "text-[#6b6459] hover:text-[#9c9486]"
              )}
            >
              {showAllOptions ? "All Options" : "Show All"}
            </button>
          </div>
        </div>

        {/* Tab content */}
        <div className="flex-1 overflow-y-auto p-5">
          {activeTab === "phases" && (
            <div className="space-y-4">
              <PhaseStrip
                phases={mode.phases}
                selectedIndex={selectedPhaseIndex}
                readOnly={isReadOnly}
                onSelect={(i) => dispatch({ type: "SELECT_PHASE", index: i })}
                onAdd={(after) => dispatch({ type: "ADD_PHASE", afterIndex: after })}
                onRemove={(i) => dispatch({ type: "REMOVE_PHASE", index: i })}
                onMove={(from, to) => dispatch({ type: "MOVE_PHASE", from, to })}
              />
              {!isReadOnly && mode.phases.length > 0 && profile.showComplianceButtons && (
                <div className="flex items-center gap-2">
                  <button
                    onClick={() =>
                      dispatch({
                        type: "ADD_COMPLIANCE_PHASE",
                        afterIndex: selectedPhaseIndex ?? mode.phases.length - 1,
                        profile: "uk_sra",
                      })
                    }
                    className="rounded-lg border border-[#2a2520] bg-[#151412] px-3 py-1.5 text-[12px] text-[#9c9486] transition-colors hover:border-amber-900/30 hover:text-[#e8e0d4]"
                  >
                    + UK SRA Check
                  </button>
                  <button
                    onClick={() =>
                      dispatch({
                        type: "ADD_COMPLIANCE_PHASE",
                        afterIndex: selectedPhaseIndex ?? mode.phases.length - 1,
                        profile: "us_prof_resp",
                      })
                    }
                    className="rounded-lg border border-[#2a2520] bg-[#151412] px-3 py-1.5 text-[12px] text-[#9c9486] transition-colors hover:border-amber-900/30 hover:text-[#e8e0d4]"
                  >
                    + US Ethics Check
                  </button>
                </div>
              )}
              {selectedPhase && selectedPhaseIndex !== null && (
                <PhaseDetail
                  phase={selectedPhase}
                  phaseNames={phaseNames}
                  readOnly={isReadOnly}
                  onChange={(patch) => dispatch({ type: "UPDATE_PHASE", index: selectedPhaseIndex, patch })}
                  profile={profile}
                />
              )}
              {!selectedPhase && mode.phases.length > 0 && (
                <div className="flex flex-col items-center rounded-xl border-2 border-dashed border-[#2a2520] py-10 text-center">
                  <p className="text-[14px] text-[#9c9486]">Select a phase above to edit</p>
                  <p className="mt-1 text-[12px] text-[#6b6459]">Click on any phase node to view its configuration</p>
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
              <pre className="min-h-[200px] rounded-xl border border-[#2a2520] bg-[#0f0e0c] p-4 font-mono text-[12px] leading-relaxed text-[#9c9486] overflow-auto">
                {JSON.stringify(mode, null, 2)}
              </pre>
              <button
                onClick={() => navigator.clipboard.writeText(JSON.stringify(mode, null, 2))}
                className="absolute right-3 top-3 rounded-lg bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#6b6459] ring-1 ring-inset ring-[#2a2520] transition-colors hover:text-[#e8e0d4] hover:bg-[#232019]"
              >
                Copy
              </button>
            </div>
          )}
        </div>

        {/* Sticky save bar */}
        {(isDirty || msg) && (
          <div className="sticky bottom-0 flex shrink-0 items-center gap-3 border-t border-[#2a2520] bg-[#0f0e0c]/95 px-5 py-3 backdrop-blur">
            {isDirty && !isReadOnly && (
              <>
                <button
                  onClick={handleDiscard}
                  disabled={busy}
                  className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-4 py-2 text-[13px] text-[#9c9486] transition-colors hover:text-[#e8e0d4] disabled:opacity-50"
                >
                  Discard
                </button>
                <button
                  onClick={handleSave}
                  disabled={busy || !mode.name.trim()}
                  className="rounded-lg bg-amber-500/20 px-4 py-2 text-[13px] font-medium text-amber-300 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/30 disabled:opacity-50"
                >
                  {busy ? "Saving..." : "Save"}
                </button>
              </>
            )}
            {msg && <span className="ml-auto text-[12px] text-[#6b6459]">{msg}</span>}
          </div>
        )}
      </div>
    </div>
  );
}
