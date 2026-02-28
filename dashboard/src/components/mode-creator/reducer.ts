import type { PipelineModeFull, PhaseConfigFull, SeedConfigFull, PhaseType } from "@/lib/types";

export interface EditorState {
  mode: PipelineModeFull;
  selectedPhaseIndex: number | null;
  expandedSeedIndex: number | null;
  activeTab: "phases" | "seeds" | "json";
  isDirty: boolean;
  isReadOnly: boolean;
  original: string; // JSON snapshot for dirty check
}

export type EditorAction =
  | { type: "LOAD_MODE"; mode: PipelineModeFull; readOnly: boolean }
  | { type: "UPDATE_MODE"; key: keyof PipelineModeFull; value: unknown }
  | { type: "SELECT_PHASE"; index: number | null }
  | { type: "UPDATE_PHASE"; index: number; patch: Partial<PhaseConfigFull> }
  | { type: "ADD_PHASE"; afterIndex: number }
  | { type: "REMOVE_PHASE"; index: number }
  | { type: "MOVE_PHASE"; from: number; to: number }
  | { type: "EXPAND_SEED"; index: number | null }
  | { type: "UPDATE_SEED"; index: number; patch: Partial<SeedConfigFull> }
  | { type: "ADD_SEED" }
  | { type: "REMOVE_SEED"; index: number }
  | { type: "SET_TAB"; tab: "phases" | "seeds" | "json" }
  | { type: "FORK"; newName: string };

export const DEFAULT_PHASE: PhaseConfigFull = {
  name: "",
  label: "",
  phase_type: "agent",
  system_prompt: "",
  instruction: "",
  error_instruction: "",
  allowed_tools: "Read,Glob,Grep,Write",
  use_docker: false,
  include_task_context: false,
  include_file_listing: false,
  runs_tests: false,
  commits: false,
  commit_message: "",
  check_artifact: null,
  allow_no_changes: false,
  next: "done",
  has_qa_fix_routing: false,
  fresh_session: false,
  fix_instruction: "",
};

export const DEFAULT_SEED: SeedConfigFull = {
  name: "",
  label: "",
  prompt: "",
  output_type: "task",
  allowed_tools: "",
  target_primary_repo: false,
};

export function blankMode(): PipelineModeFull {
  return {
    name: "",
    label: "",
    category: "",
    phases: [
      { ...DEFAULT_PHASE, name: "backlog", label: "Backlog", phase_type: "setup" as PhaseType, next: "done" },
    ],
    seed_modes: [],
    initial_status: "backlog",
    uses_git_worktrees: true,
    uses_docker: true,
    uses_test_cmd: false,
    integration: "git_pr",
    default_max_attempts: 5,
  };
}

function cloneMode(m: PipelineModeFull): PipelineModeFull {
  return JSON.parse(JSON.stringify(m));
}

function markDirty(state: EditorState, mode: PipelineModeFull): EditorState {
  return { ...state, mode, isDirty: JSON.stringify(mode) !== state.original };
}

export function editorReducer(state: EditorState, action: EditorAction): EditorState {
  switch (action.type) {
    case "LOAD_MODE": {
      const snap = JSON.stringify(action.mode);
      return {
        mode: cloneMode(action.mode),
        selectedPhaseIndex: null,
        expandedSeedIndex: null,
        activeTab: "phases",
        isDirty: false,
        isReadOnly: action.readOnly,
        original: snap,
      };
    }

    case "UPDATE_MODE": {
      const mode = { ...state.mode, [action.key]: action.value } as PipelineModeFull;
      return markDirty(state, mode);
    }

    case "SELECT_PHASE":
      return { ...state, selectedPhaseIndex: action.index };

    case "UPDATE_PHASE": {
      const phases = [...state.mode.phases];
      phases[action.index] = { ...phases[action.index], ...action.patch };
      return markDirty(state, { ...state.mode, phases });
    }

    case "ADD_PHASE": {
      const phases = [...state.mode.phases];
      const insertAt = action.afterIndex + 1;
      const prevPhase = phases[action.afterIndex];
      const nextName = prevPhase?.next || "done";

      const newPhase: PhaseConfigFull = {
        ...DEFAULT_PHASE,
        name: `phase_${Date.now() % 10000}`,
        label: "New Phase",
        use_docker: state.mode.uses_docker,
        next: nextName,
      };

      // Point the previous phase to the new one
      if (prevPhase) {
        phases[action.afterIndex] = { ...prevPhase, next: newPhase.name };
      }

      phases.splice(insertAt, 0, newPhase);
      const mode = { ...state.mode, phases };
      return { ...markDirty(state, mode), selectedPhaseIndex: insertAt };
    }

    case "REMOVE_PHASE": {
      const phases = [...state.mode.phases];
      const removed = phases[action.index];
      phases.splice(action.index, 1);

      // Fix up any phase that pointed to the removed one
      for (let i = 0; i < phases.length; i++) {
        if (phases[i].next === removed.name) {
          phases[i] = { ...phases[i], next: removed.next };
        }
      }

      const sel = state.selectedPhaseIndex === action.index ? null : state.selectedPhaseIndex;
      return { ...markDirty(state, { ...state.mode, phases }), selectedPhaseIndex: sel };
    }

    case "MOVE_PHASE": {
      const phases = [...state.mode.phases];
      const [moved] = phases.splice(action.from, 1);
      phases.splice(action.to, 0, moved);

      // Re-chain next pointers based on new order
      for (let i = 0; i < phases.length; i++) {
        phases[i] = { ...phases[i], next: i < phases.length - 1 ? phases[i + 1].name : "done" };
      }

      return { ...markDirty(state, { ...state.mode, phases }), selectedPhaseIndex: action.to };
    }

    case "EXPAND_SEED":
      return { ...state, expandedSeedIndex: action.index };

    case "UPDATE_SEED": {
      const seeds = [...state.mode.seed_modes];
      seeds[action.index] = { ...seeds[action.index], ...action.patch };
      return markDirty(state, { ...state.mode, seed_modes: seeds });
    }

    case "ADD_SEED": {
      const seeds = [...state.mode.seed_modes, { ...DEFAULT_SEED }];
      const mode = { ...state.mode, seed_modes: seeds };
      return { ...markDirty(state, mode), expandedSeedIndex: seeds.length - 1 };
    }

    case "REMOVE_SEED": {
      const seeds = [...state.mode.seed_modes];
      seeds.splice(action.index, 1);
      const exp = state.expandedSeedIndex === action.index ? null : state.expandedSeedIndex;
      return { ...markDirty(state, { ...state.mode, seed_modes: seeds }), expandedSeedIndex: exp };
    }

    case "SET_TAB":
      return { ...state, activeTab: action.tab };

    case "FORK": {
      const mode = cloneMode(state.mode);
      mode.name = action.newName;
      mode.label = action.newName;
      const snap = JSON.stringify(mode);
      return {
        ...state,
        mode,
        isReadOnly: false,
        isDirty: true,
        original: snap,
      };
    }

    default:
      return state;
  }
}

export const INITIAL_STATE: EditorState = {
  mode: blankMode(),
  selectedPhaseIndex: null,
  expandedSeedIndex: null,
  activeTab: "phases",
  isDirty: false,
  isReadOnly: false,
  original: "",
};
