import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useFocus, setFocus, clearFocus } from "@/lib/api";
import { Target, X, ChevronRight } from "lucide-react";

export function FocusPicker() {
  const { data: focus } = useFocus();
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  const queryClient = useQueryClient();

  async function handleSet(e: React.FormEvent) {
    e.preventDefault();
    if (!text.trim()) return;
    setSaving(true);
    try {
      await setFocus(text.trim());
      queryClient.invalidateQueries({ queryKey: ["focus"] });
      setText("");
      setOpen(false);
    } finally {
      setSaving(false);
    }
  }

  async function handleClear() {
    await clearFocus();
    queryClient.invalidateQueries({ queryKey: ["focus"] });
    setOpen(false);
  }

  if (focus?.active) {
    return (
      <div className="flex items-center gap-1">
        <button
          onClick={() => setOpen((v) => !v)}
          className="flex items-center gap-1.5 rounded-md bg-amber-500/10 px-2 py-1 text-[11px] font-medium text-amber-400 ring-1 ring-inset ring-amber-500/20 hover:bg-amber-500/15 transition-colors max-w-[180px]"
          title={focus.text}
        >
          <Target className="h-3 w-3 shrink-0" />
          <span className="truncate">{focus.text}</span>
        </button>
        <button
          onClick={handleClear}
          className="rounded p-0.5 text-zinc-600 hover:text-zinc-400 transition-colors"
          title="Clear focus"
        >
          <X className="h-3 w-3" />
        </button>
        {open && (
          <FocusModal
            initial={focus.text}
            onSubmit={handleSet}
            text={text}
            setText={setText}
            saving={saving}
            onClose={() => setOpen(false)}
          />
        )}
      </div>
    );
  }

  return (
    <>
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1 rounded px-1.5 py-1 text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors"
        title="Set focus area"
      >
        <Target className="h-3 w-3" />
        <span>Focus</span>
        <ChevronRight className="h-2.5 w-2.5" />
      </button>
      {open && (
        <FocusModal
          onSubmit={handleSet}
          text={text}
          setText={setText}
          saving={saving}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

function FocusModal({
  initial,
  onSubmit,
  text,
  setText,
  saving,
  onClose,
}: {
  initial?: string;
  onSubmit: (e: React.FormEvent) => void;
  text: string;
  setText: (v: string) => void;
  saving: boolean;
  onClose: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 pt-[15vh]" onClick={onClose}>
      <form
        onClick={(e) => e.stopPropagation()}
        onSubmit={onSubmit}
        className="w-full max-w-md rounded-lg border border-white/[0.08] bg-zinc-900 p-5 shadow-2xl"
      >
        <div className="mb-3 flex items-center justify-between">
          <div>
            <h2 className="text-sm font-semibold text-zinc-200">Set Focus Area</h2>
            <p className="mt-0.5 text-[11px] text-zinc-500">Seed scans will bias toward this area while active</p>
          </div>
          <button type="button" onClick={onClose} className="text-zinc-500 hover:text-zinc-300">
            <X className="h-4 w-4" />
          </button>
        </div>
        {initial && (
          <p className="mb-2 text-[11px] text-zinc-500">Current: <span className="text-zinc-300">{initial}</span></p>
        )}
        <textarea
          autoFocus
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="e.g. improve pipeline error handling and observability"
          rows={3}
          className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 placeholder-zinc-600 outline-none focus:border-amber-500/40 resize-none"
        />
        <div className="mt-3 flex justify-end gap-2">
          <button type="button" onClick={onClose} className="rounded-md px-3 py-1.5 text-[12px] text-zinc-400 hover:text-zinc-200">
            Cancel
          </button>
          <button
            type="submit"
            disabled={saving || !text.trim()}
            className="rounded-md bg-amber-500/15 px-4 py-1.5 text-[12px] font-medium text-amber-400 ring-1 ring-inset ring-amber-500/20 hover:bg-amber-500/25 disabled:opacity-50 transition-colors"
          >
            {saving ? "Setting…" : "Set Focus"}
          </button>
        </div>
      </form>
    </div>
  );
}
