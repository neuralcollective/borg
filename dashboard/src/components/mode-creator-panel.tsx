import { useMemo, useState } from "react";
import { removeCustomMode, saveCustomMode, useCustomModes, useFullModes } from "@/lib/api";
import type { PipelineModeFull } from "@/lib/types";

function cloneMode(mode: PipelineModeFull): PipelineModeFull {
  return JSON.parse(JSON.stringify(mode)) as PipelineModeFull;
}

function pretty(mode: PipelineModeFull): string {
  return JSON.stringify(mode, null, 2);
}

function parseMode(text: string): PipelineModeFull {
  return JSON.parse(text) as PipelineModeFull;
}

export function ModeCreatorPanel() {
  const { data: allModes = [], refetch: refetchAll } = useFullModes();
  const { data: customModes = [], refetch: refetchCustom } = useCustomModes();
  const [templateName, setTemplateName] = useState<string>("sweborg");
  const [modeName, setModeName] = useState("");
  const [editorText, setEditorText] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string>("");

  const template = useMemo(
    () => allModes.find((m) => m.name === templateName) ?? allModes[0] ?? null,
    [allModes, templateName]
  );
  const customNameSet = useMemo(
    () => new Set(customModes.map((m) => m.name)),
    [customModes]
  );
  const builtInModes = useMemo(
    () => allModes.filter((m) => !customNameSet.has(m.name)),
    [allModes, customNameSet]
  );

  function loadTemplate() {
    if (!template) return;
    const next = cloneMode(template);
    if (modeName.trim()) {
      next.name = modeName.trim();
      next.label = modeName.trim();
    }
    setEditorText(pretty(next));
    setMsg("");
  }

  async function handleSave() {
    if (busy) return;
    setBusy(true);
    setMsg("");
    try {
      const parsed = parseMode(editorText);
      if (modeName.trim()) {
        parsed.name = modeName.trim();
        if (!parsed.label?.trim()) parsed.label = modeName.trim();
      }
      await saveCustomMode(parsed);
      setModeName(parsed.name);
      setEditorText(pretty(parsed));
      await Promise.all([refetchAll(), refetchCustom()]);
      setMsg(`Saved mode '${parsed.name}'`);
    } catch (err) {
      const text = err instanceof Error ? err.message : "save failed";
      setMsg(`Save failed (${text})`);
    } finally {
      setBusy(false);
    }
  }

  async function handleDelete(name: string) {
    if (busy) return;
    setBusy(true);
    setMsg("");
    try {
      await removeCustomMode(name);
      await Promise.all([refetchAll(), refetchCustom()]);
      setMsg(`Deleted mode '${name}'`);
      if (modeName === name) setModeName("");
    } catch (err) {
      const text = err instanceof Error ? err.message : "delete failed";
      setMsg(`Delete failed (${text})`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full min-h-0">
      <div className="w-[300px] shrink-0 border-r border-white/[0.06] p-3">
        <div className="text-[11px] font-medium uppercase tracking-wide text-zinc-500">Custom Borg Creator</div>
        <div className="mt-3 space-y-2">
          <label className="block text-[11px] text-zinc-500">Mode Name</label>
          <input
            value={modeName}
            onChange={(e) => setModeName(e.target.value)}
            placeholder="myborg"
            className="w-full rounded border border-white/[0.08] bg-white/[0.03] px-2 py-1.5 text-[12px] text-zinc-200 outline-none"
          />

          <label className="mt-2 block text-[11px] text-zinc-500">Template</label>
          <select
            value={templateName}
            onChange={(e) => setTemplateName(e.target.value)}
            className="w-full rounded border border-white/[0.08] bg-white/[0.03] px-2 py-1.5 text-[12px] text-zinc-300 outline-none"
          >
            {builtInModes.length > 0 && (
              <optgroup label="Built-in Modes">
                {builtInModes.map((m) => (
                  <option key={m.name} value={m.name}>
                    {m.name}
                  </option>
                ))}
              </optgroup>
            )}
            {customModes.length > 0 && (
              <optgroup label="Custom Modes">
                {customModes.map((m) => (
                  <option key={m.name} value={m.name}>
                    {m.name}
                  </option>
                ))}
              </optgroup>
            )}
          </select>
          <button
            onClick={loadTemplate}
            className="w-full rounded bg-white/[0.06] px-2 py-1.5 text-[12px] text-zinc-300 hover:bg-white/[0.1]"
          >
            Load Template Into Editor
          </button>
        </div>

        <div className="mt-4 border-t border-white/[0.06] pt-3">
          <div className="mb-2 text-[11px] text-zinc-500">Existing Custom Modes</div>
          <div className="space-y-1">
            {customModes.map((m) => (
              <div key={m.name} className="flex items-center gap-1">
                <button
                  onClick={() => {
                    setModeName(m.name);
                    setEditorText(pretty(m));
                    setMsg("");
                  }}
                  className="flex-1 rounded bg-white/[0.04] px-2 py-1 text-left text-[11px] text-zinc-300 hover:bg-white/[0.08]"
                >
                  {m.name}
                </button>
                <button
                  onClick={() => handleDelete(m.name)}
                  className="rounded bg-red-500/20 px-2 py-1 text-[11px] text-red-300 hover:bg-red-500/30"
                >
                  Del
                </button>
              </div>
            ))}
            {customModes.length === 0 && (
              <div className="text-[11px] text-zinc-600">No custom modes yet.</div>
            )}
          </div>
        </div>
      </div>

      <div className="flex min-w-0 flex-1 flex-col p-3">
        <div className="mb-2 text-[11px] text-zinc-500">
          Edit full pipeline JSON: phases, prompts, tools, integration, seeds.
        </div>
        <textarea
          value={editorText}
          onChange={(e) => setEditorText(e.target.value)}
          placeholder="Load a template, then edit JSON..."
          className="min-h-0 flex-1 resize-none rounded border border-white/[0.08] bg-black/30 p-3 font-mono text-[11px] text-zinc-200 outline-none"
        />
        <div className="mt-2 flex items-center gap-2">
          <button
            onClick={handleSave}
            disabled={busy || !editorText.trim()}
            className="rounded bg-blue-500/20 px-3 py-1.5 text-[12px] text-blue-300 disabled:cursor-not-allowed disabled:text-zinc-600"
          >
            Save Custom Mode
          </button>
          {msg && <span className="text-[11px] text-zinc-500">{msg}</span>}
        </div>
      </div>
    </div>
  );
}
