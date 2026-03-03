import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { checkConflicts, createProject, createTask, uploadProjectFiles, useProjects } from "@/lib/api";
import type { ConflictHit } from "@/lib/api";
import type { Project } from "@/lib/types";
import { AlertTriangle, Scale, X, Paperclip, ChevronDown } from "lucide-react";

const TASK_TYPES = [
  { value: "research_memo", label: "Research Memo" },
  { value: "case_brief", label: "Case Brief" },
  { value: "demand_letter", label: "Demand Letter" },
  { value: "contract_analysis", label: "Contract Analysis" },
  { value: "contract_review", label: "Contract Review" },
  { value: "nda_triage", label: "NDA Triage" },
  { value: "motion_brief", label: "Motion / Brief" },
  { value: "regulatory_analysis", label: "Regulatory Analysis" },
  { value: "compliance", label: "Compliance Review" },
  { value: "risk_assessment", label: "Risk Assessment" },
  { value: "vendor_check", label: "Vendor Check" },
  { value: "meeting_briefing", label: "Meeting Briefing" },
] as const;

type TaskTypeValue = typeof TASK_TYPES[number]["value"];

const JURISDICTION_SUGGESTIONS = [
  "US Federal",
  "California",
  "New York",
  "Texas",
  "Florida",
  "Illinois",
  "Massachusetts",
  "Washington",
  "UK",
  "EU",
  "Canada",
  "Australia",
];

function buildTitle(taskType: TaskTypeValue, clientParty: string, opposingParty: string): string {
  const label = TASK_TYPES.find((t) => t.value === taskType)?.label ?? "Task";
  const parts: string[] = [label];
  if (clientParty.trim()) parts.push(`— ${clientParty.trim()}`);
  if (opposingParty.trim()) parts.push(`v. ${opposingParty.trim()}`);
  return parts.join(" ");
}

function buildDescription(opts: {
  taskType: TaskTypeValue;
  jurisdiction: string;
  clientParty: string;
  opposingParty: string;
  questionFacts: string;
  deadline: string;
  privileged: boolean;
}): string {
  const label = TASK_TYPES.find((t) => t.value === opts.taskType)?.label ?? "Task";
  const lines: string[] = [];

  lines.push(`Task Type: ${label}`);
  if (opts.jurisdiction.trim()) lines.push(`Jurisdiction: ${opts.jurisdiction.trim()}`);
  if (opts.clientParty.trim()) lines.push(`Client / Your Party: ${opts.clientParty.trim()}`);
  if (opts.opposingParty.trim()) lines.push(`Opposing Party: ${opts.opposingParty.trim()}`);
  if (opts.deadline) lines.push(`Deadline: ${opts.deadline}`);
  if (opts.privileged) lines.push(`Privilege: Attorney Work Product — Privileged & Confidential`);

  if (opts.questionFacts.trim()) {
    lines.push("");
    lines.push("Question Presented / Key Facts:");
    lines.push(opts.questionFacts.trim());
  }

  return lines.join("\n");
}

interface JurisdictionInputProps {
  value: string;
  onChange: (v: string) => void;
}

function JurisdictionInput({ value, onChange }: JurisdictionInputProps) {
  const [open, setOpen] = useState(false);
  const filtered = JURISDICTION_SUGGESTIONS.filter(
    (j) => j.toLowerCase().includes(value.toLowerCase()) && j.toLowerCase() !== value.toLowerCase()
  );

  return (
    <div className="relative">
      <input
        value={value}
        onChange={(e) => { onChange(e.target.value); setOpen(true); }}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 150)}
        placeholder="e.g. US Federal, California, UK"
        className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 placeholder-zinc-600 outline-none focus:border-blue-500/40"
      />
      {open && filtered.length > 0 && (
        <div className="absolute z-20 mt-1 w-full rounded-md border border-white/[0.08] bg-zinc-900 py-1 shadow-xl">
          {filtered.map((j) => (
            <button
              key={j}
              type="button"
              onMouseDown={() => { onChange(j); setOpen(false); }}
              className="w-full px-3 py-1.5 text-left text-[12px] text-zinc-300 hover:bg-white/[0.06]"
            >
              {j}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

interface MatterSelectorProps {
  matters: Project[];
  selectedId: number | "new" | null;
  onChange: (v: number | "new") => void;
  newName: string;
  onNewName: (v: string) => void;
  newClient: string;
  onNewClient: (v: string) => void;
}

function MatterSelector({
  matters,
  selectedId,
  onChange,
  newName,
  onNewName,
  newClient,
  onNewClient,
}: MatterSelectorProps) {
  return (
    <div className="space-y-2">
      <div className="relative">
        <select
          value={selectedId === "new" ? "new" : (selectedId ?? "")}
          onChange={(e) => {
            const v = e.target.value;
            onChange(v === "new" ? "new" : Number(v));
          }}
          className="w-full appearance-none rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 pr-8 text-[13px] text-zinc-200 outline-none focus:border-blue-500/40"
        >
          <option value="" disabled>Select a matter…</option>
          {matters.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}{m.client_name ? ` — ${m.client_name}` : ""}
            </option>
          ))}
          <option value="new">+ New Matter</option>
        </select>
        <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-zinc-500" />
      </div>

      {selectedId === "new" && (
        <div className="space-y-2 rounded-md border border-white/[0.06] bg-white/[0.02] p-3">
          <input
            value={newName}
            onChange={(e) => onNewName(e.target.value)}
            placeholder="Matter name (e.g. Smith v. Jones)"
            className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 placeholder-zinc-600 outline-none focus:border-blue-500/40"
          />
          <input
            value={newClient}
            onChange={(e) => onNewClient(e.target.value)}
            placeholder="Client name (optional)"
            className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 placeholder-zinc-600 outline-none focus:border-blue-500/40"
          />
        </div>
      )}
    </div>
  );
}

export function LegalTaskCreator() {
  const [open, setOpen] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");

  const [selectedMatter, setSelectedMatter] = useState<number | "new" | null>(null);
  const [newMatterName, setNewMatterName] = useState("");
  const [newMatterClient, setNewMatterClient] = useState("");

  const [taskType, setTaskType] = useState<TaskTypeValue>("research_memo");
  const [jurisdiction, setJurisdiction] = useState("");
  const [clientParty, setClientParty] = useState("");
  const [opposingParty, setOpposingParty] = useState("");
  const [questionFacts, setQuestionFacts] = useState("");
  const [deadline, setDeadline] = useState("");
  const [privileged, setPrivileged] = useState(false);
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  const [conflicts, setConflicts] = useState<ConflictHit[]>([]);
  const [conflictAcked, setConflictAcked] = useState(false);

  const fileInputRef = useRef<HTMLInputElement>(null);
  const conflictTimer = useRef<ReturnType<typeof setTimeout>>(null);
  const queryClient = useQueryClient();
  const { data: allProjects = [] } = useProjects();

  const runConflictCheck = useCallback((client: string, opposing: string) => {
    if (conflictTimer.current) clearTimeout(conflictTimer.current);
    conflictTimer.current = setTimeout(async () => {
      if (!client.trim() && !opposing.trim()) {
        setConflicts([]);
        return;
      }
      try {
        const hits = await checkConflicts(client.trim(), opposing.trim());
        setConflicts(hits);
        setConflictAcked(false);
      } catch {
        setConflicts([]);
      }
    }, 500);
  }, []);

  useEffect(() => {
    runConflictCheck(clientParty || newMatterClient, opposingParty);
  }, [clientParty, opposingParty, newMatterClient, runConflictCheck]);

  const legalMatters = allProjects.filter(
    (p) => p.mode === "lawborg" || p.mode === "legal"
  );

  function resetForm() {
    setSelectedMatter(null);
    setNewMatterName("");
    setNewMatterClient("");
    setTaskType("research_memo");
    setJurisdiction("");
    setClientParty("");
    setOpposingParty("");
    setQuestionFacts("");
    setDeadline("");
    setPrivileged(false);
    setPendingFiles([]);
    setConflicts([]);
    setConflictAcked(false);
    setError("");
  }

  function handleClose() {
    setOpen(false);
    resetForm();
  }

  function handleFileChange(e: React.ChangeEvent<HTMLInputElement>) {
    if (!e.target.files) return;
    setPendingFiles((prev) => [...prev, ...Array.from(e.target.files!)]);
    e.target.value = "";
  }

  function removeFile(idx: number) {
    setPendingFiles((prev) => prev.filter((_, i) => i !== idx));
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();

    if (selectedMatter === null) {
      setError("Please select a matter or create a new one.");
      return;
    }
    if (selectedMatter === "new" && !newMatterName.trim()) {
      setError("Matter name is required.");
      return;
    }
    if (!questionFacts.trim()) {
      setError("Question presented / key facts is required.");
      return;
    }
    if (conflicts.length > 0 && !conflictAcked) {
      setError("Please acknowledge the conflict of interest warning before proceeding.");
      return;
    }

    setSubmitting(true);
    setError("");

    try {
      let projectId: number;

      if (selectedMatter === "new") {
        const created = await createProject(newMatterName.trim(), "lawborg", {
          client_name: (newMatterClient.trim() || clientParty.trim()) || undefined,
          opposing_counsel: opposingParty.trim() || undefined,
          jurisdiction: jurisdiction.trim() || undefined,
          matter_type: TASK_TYPES.find((t) => t.value === taskType)?.label,
          privilege_level: privileged ? "attorney_work_product" : undefined,
        });
        projectId = created.id;
        queryClient.invalidateQueries({ queryKey: ["projects"] });
      } else {
        projectId = selectedMatter;
      }

      const title = buildTitle(taskType, clientParty, opposingParty);
      const description = buildDescription({
        taskType,
        jurisdiction,
        clientParty,
        opposingParty,
        questionFacts,
        deadline,
        privileged,
      });

      const { id: taskId } = await createTask(title, description, "lawborg", undefined, projectId, taskType);

      if (pendingFiles.length > 0) {
        try {
          await uploadProjectFiles(projectId, pendingFiles);
        } catch {
          // non-fatal — task created, files failed
        }
      }

      queryClient.invalidateQueries({ queryKey: ["tasks"] });
      queryClient.invalidateQueries({ queryKey: ["projects"] });
      handleClose();
      return taskId;
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create task");
    } finally {
      setSubmitting(false);
    }
  }

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="inline-flex items-center gap-1.5 rounded-md bg-indigo-500/15 px-3 py-1.5 text-[11px] font-medium text-indigo-400 ring-1 ring-inset ring-indigo-500/20 transition-colors hover:bg-indigo-500/25"
      >
        <Scale className="h-3 w-3" />
        New Legal Task
      </button>
    );
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/60 px-4 py-[10vh]"
      onClick={handleClose}
    >
      <form
        onClick={(e) => e.stopPropagation()}
        onSubmit={handleSubmit}
        className="w-full max-w-xl rounded-lg border border-white/[0.08] bg-zinc-900 p-5 shadow-2xl"
      >
        <div className="mb-5 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Scale className="h-4 w-4 text-indigo-400" />
            <h2 className="text-sm font-semibold text-zinc-200">New Legal Task</h2>
          </div>
          <button type="button" onClick={handleClose} className="text-zinc-500 hover:text-zinc-300">
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-4">
          {/* Matter */}
          <div>
            <label className="mb-1.5 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">
              Matter
            </label>
            <MatterSelector
              matters={legalMatters}
              selectedId={selectedMatter}
              onChange={setSelectedMatter}
              newName={newMatterName}
              onNewName={setNewMatterName}
              newClient={newMatterClient}
              onNewClient={setNewMatterClient}
            />
          </div>

          {/* Task type */}
          <div>
            <label className="mb-1.5 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">
              Task Type
            </label>
            <div className="flex flex-wrap gap-1.5">
              {TASK_TYPES.map((t) => (
                <button
                  key={t.value}
                  type="button"
                  onClick={() => setTaskType(t.value)}
                  className={
                    taskType === t.value
                      ? "rounded-full border border-indigo-500/40 bg-indigo-500/20 px-3 py-1 text-[11px] font-medium text-indigo-300 transition-colors"
                      : "rounded-full border border-white/[0.08] px-3 py-1 text-[11px] text-zinc-400 transition-colors hover:border-white/[0.15] hover:text-zinc-200"
                  }
                >
                  {t.label}
                </button>
              ))}
            </div>
          </div>

          {/* Jurisdiction */}
          <div>
            <label className="mb-1.5 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">
              Jurisdiction
            </label>
            <JurisdictionInput value={jurisdiction} onChange={setJurisdiction} />
          </div>

          {/* Parties */}
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1.5 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">
                Client / Your Party
              </label>
              <input
                value={clientParty}
                onChange={(e) => setClientParty(e.target.value)}
                placeholder="e.g. Acme Corp."
                className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 placeholder-zinc-600 outline-none focus:border-blue-500/40"
              />
            </div>
            <div>
              <label className="mb-1.5 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">
                Opposing Party
              </label>
              <input
                value={opposingParty}
                onChange={(e) => setOpposingParty(e.target.value)}
                placeholder="e.g. XYZ LLC"
                className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 placeholder-zinc-600 outline-none focus:border-blue-500/40"
              />
            </div>
          </div>

          {/* Question / facts */}
          <div>
            <label className="mb-1.5 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">
              Question Presented / Key Facts
            </label>
            <textarea
              autoFocus
              value={questionFacts}
              onChange={(e) => setQuestionFacts(e.target.value)}
              placeholder="Describe the legal question, key facts, or specific analysis needed…"
              rows={5}
              className="w-full resize-none rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 placeholder-zinc-600 outline-none focus:border-blue-500/40"
            />
          </div>

          {/* Deadline + privilege row */}
          <div className="flex items-end gap-4">
            <div className="flex-1">
              <label className="mb-1.5 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">
                Deadline
              </label>
              <input
                type="date"
                value={deadline}
                onChange={(e) => setDeadline(e.target.value)}
                className="w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-3 py-2 text-[13px] text-zinc-200 outline-none focus:border-blue-500/40 [color-scheme:dark]"
              />
            </div>
            <label className="flex cursor-pointer items-center gap-2 pb-2">
              <div
                onClick={() => setPrivileged((v) => !v)}
                className={`relative h-4 w-7 rounded-full transition-colors ${privileged ? "bg-indigo-500/60" : "bg-white/[0.1]"}`}
              >
                <div
                  className={`absolute top-0.5 h-3 w-3 rounded-full bg-white shadow transition-transform ${privileged ? "translate-x-3" : "translate-x-0.5"}`}
                />
              </div>
              <span className="text-[11px] text-zinc-400">Attorney Work Product — Privileged</span>
            </label>
          </div>

          {/* File attachments */}
          <div>
            <label className="mb-1.5 block text-[10px] font-medium uppercase tracking-wider text-zinc-500">
              Attachments
            </label>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              onChange={handleFileChange}
              className="hidden"
            />
            {pendingFiles.length > 0 && (
              <div className="mb-2 space-y-1 rounded-md border border-white/[0.06] bg-black/20 p-2">
                {pendingFiles.map((f, idx) => (
                  <div key={idx} className="flex items-center justify-between text-[11px] text-zinc-400">
                    <span className="truncate pr-2">{f.name}</span>
                    <button
                      type="button"
                      onClick={() => removeFile(idx)}
                      className="shrink-0 text-zinc-600 hover:text-zinc-300"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </div>
                ))}
              </div>
            )}
            <button
              type="button"
              onClick={() => fileInputRef.current?.click()}
              className="inline-flex items-center gap-1.5 rounded-md border border-white/[0.08] bg-white/[0.03] px-3 py-1.5 text-[12px] text-zinc-400 transition-colors hover:bg-white/[0.07] hover:text-zinc-200"
            >
              <Paperclip className="h-3 w-3" />
              Attach files
            </button>
          </div>
        </div>

        {conflicts.length > 0 && !conflictAcked && (
          <div className="mt-3 rounded-md border border-amber-500/30 bg-amber-500/10 p-3">
            <div className="mb-1.5 flex items-center gap-1.5 text-[12px] font-medium text-amber-400">
              <AlertTriangle className="h-3.5 w-3.5" />
              Potential Conflict of Interest
            </div>
            <div className="space-y-1">
              {conflicts.map((c, i) => (
                <p key={i} className="text-[11px] text-amber-300/80">
                  <span className="font-medium">{c.party_name}</span>
                  {" "}({c.party_role === "opposing_counsel" ? "opposing" : c.party_role})
                  {" in "}<span className="font-medium">{c.project_name}</span>
                  {" — matched via "}{c.matched_field === "client_name" ? "client" : "opposing counsel"}
                </p>
              ))}
            </div>
            <button
              type="button"
              onClick={() => setConflictAcked(true)}
              className="mt-2 rounded-md bg-amber-500/20 px-3 py-1 text-[11px] font-medium text-amber-400 ring-1 ring-inset ring-amber-500/20 hover:bg-amber-500/30"
            >
              Acknowledge &amp; Continue
            </button>
          </div>
        )}

        {error && <p className="mt-3 text-[11px] text-red-400">{error}</p>}

        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={handleClose}
            className="rounded-md px-3 py-1.5 text-[12px] text-zinc-400 hover:text-zinc-200"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={submitting}
            className="rounded-md bg-indigo-500/20 px-4 py-1.5 text-[12px] font-medium text-indigo-400 ring-1 ring-inset ring-indigo-500/20 transition-colors hover:bg-indigo-500/30 disabled:opacity-50"
          >
            {submitting ? "Creating…" : "Create Task"}
          </button>
        </div>
      </form>
    </div>
  );
}
