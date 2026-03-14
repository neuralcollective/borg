export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export type JsonObject = Record<string, Json>;

// --- Projects ---

export type CreateProjectBody = {
  name: string;
  mode?: string;
  client_name?: string;
  jurisdiction?: string;
  matter_type?: string;
  privilege_level?: string;
};

export type UpdateProjectBody = {
  name?: string;
  client_name?: string;
  case_number?: string;
  jurisdiction?: string;
  matter_type?: string;
  opposing_counsel?: string;
  deadline?: string | null;
  privilege_level?: string;
  status?: string;
  default_template_id?: number | null;
};

export type ProjectTaskCounts = {
  total: number;
  pending: number;
  running: number;
  done: number;
  failed: number;
};

export type Project = {
  id: number;
  name: string;
  mode: string;
  repo_path: string;
  client_name: string;
  case_number: string;
  jurisdiction: string;
  matter_type: string;
  opposing_counsel: string;
  deadline: string | null;
  privilege_level: string;
  status: string;
  default_template_id: number | null;
  session_privileged: boolean;
  created_at: string;
  task_counts?: ProjectTaskCounts;
};

// --- Tasks ---

export type CreateTaskBody = {
  title: string;
  description?: string;
  mode?: string;
  repo?: string;
  project_id?: number;
  task_type?: string;
  requires_exhaustive_corpus_review?: boolean;
  notify_chat?: string;
  chat_thread?: string;
};

export type PatchTaskBody = {
  title?: string;
  description?: string;
};

export type TaskOutput = {
  id: number;
  task_id: number;
  phase: string;
  output: string;
  exit_code: number;
  created_at: string;
};

export type TaskMessage = {
  id: number;
  task_id: number;
  role: string;
  content: string;
  created_at: string;
  delivered_phase?: string;
};

export type Task = {
  id: number;
  title: string;
  description: string;
  repo_path: string;
  branch: string;
  status: string;
  attempt: number;
  max_attempts: number;
  last_error: string;
  mode: string;
  project_id: number;
  task_type: string;
  requires_exhaustive_corpus_review: boolean;
  review_status?: string | null;
  revision_count: number;
  outputs?: TaskOutput[];
  structured_data?: Json;
};

// --- Files & Uploads ---

export type CreateUploadSessionBody = {
  file_name: string;
  mime_type?: string;
  file_size: number;
  chunk_size: number;
  total_chunks: number;
  is_zip?: boolean;
  privileged?: boolean;
};

export type UploadSession = {
  session_id: number;
};

export type ProjectFile = {
  id: number;
  project_id: number;
  file_name: string;
  source_path: string;
  mime_type: string;
  size_bytes: number;
  privileged: boolean;
  has_text: boolean;
  text_chars: number;
  created_at: string;
};

export type ProjectFilesSummary = {
  total_files: number;
  text_files: number;
  privileged_files?: number;
  total_bytes?: number;
};

export type ProjectFilesResponse = {
  total?: number;
  summary?: ProjectFilesSummary;
  items?: ProjectFile[];
};

// --- Documents ---

export type ProjectDocument = {
  task_id: number;
  branch: string;
  path: string;
  repo_slug: string;
  task_title: string;
  task_status: string;
};

// --- Chat ---

export type ChatPostBody = {
  text: string;
  sender?: string;
  thread?: string;
  model?: string;
};

export type ChatMessage = {
  role: "assistant" | "user";
  sender?: string;
  text: string;
  ts?: number | string;
  thread?: string;
  raw_stream?: string;
};

// --- Search ---

export type SearchResult = {
  id?: number;
  file_name?: string;
  snippet?: string;
  score?: number;
  project_id?: number;
};

// --- Knowledge ---

export type KnowledgeFile = {
  id: number;
  file_name: string;
  description?: string;
  inline?: boolean;
  tags?: string;
  category?: string;
  jurisdiction?: string;
  has_text?: boolean;
  text_chars?: number;
  created_at?: string;
};

export type UpdateKnowledgeBody = {
  description?: string;
  inline?: boolean;
  tags?: string;
  category?: string;
  jurisdiction?: string;
};

// --- System ---

export type SystemStatus = {
  running: number;
  queued: number;
};

// --- Shares ---

export type ProjectShare = {
  user_id: number;
  role: string;
};

export type ShareLink = {
  id: number;
  token: string;
  expires_at?: string;
};

// --- Revisions ---

export type RevisionEntry = {
  id: number;
  task_id: number;
  feedback: string;
  created_at: string;
};

// --- SDK helpers ---

export type UploadedFile = {
  relative_path: string;
  mime_type: string;
  size_bytes: number;
  expected_text: boolean;
};

export type TokenSource =
  | { kind: "explicit"; token: string }
  | { kind: "env"; name: string }
  | { kind: "file"; path: string }
  | { kind: "auto"; endpoint: string };
