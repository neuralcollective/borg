export {
  BorgClient,
  BorgError,
  DEFAULT_BASE_URL,
  DEFAULT_POLL_INTERVAL_MS,
  DEFAULT_TIMEOUT_MS,
  UPLOAD_CHUNK_SIZE,
} from "./client.js";
export type { BorgClientConfig } from "./client.js";

export { guessMimeType, isExpectedTextFile, uploadDirectory, waitForIngestion } from "./upload.js";

export type {
  ChatMessage,
  ChatPostBody,
  CreateProjectBody,
  CreateTaskBody,
  CreateUploadSessionBody,
  Json,
  JsonObject,
  KnowledgeFile,
  PatchTaskBody,
  Project,
  ProjectDocument,
  ProjectFile,
  ProjectFilesResponse,
  ProjectFilesSummary,
  ProjectTaskCounts,
  RevisionEntry,
  SearchResult,
  ShareLink,
  ProjectShare,
  SystemStatus,
  Task,
  TaskMessage,
  TaskOutput,
  TokenSource,
  UpdateKnowledgeBody,
  UpdateProjectBody,
  UploadedFile,
  UploadSession,
} from "./types.js";
