#!/usr/bin/env node
// Borg internal MCP server — exposes BorgSearch (document search/retrieval)
// and pipeline task management to the chat agent.

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { ListToolsRequestSchema, CallToolRequestSchema } from "@modelcontextprotocol/sdk/types.js";

const API_URL = process.env.API_BASE_URL || "http://127.0.0.1:3131";
const API_TOKEN = process.env.API_TOKEN || "";
const PROJECT_ID = process.env.PROJECT_ID || "";
const PROJECT_MODE = process.env.PROJECT_MODE || "";
const CHAT_THREAD = process.env.CHAT_THREAD || "";
const WORKSPACE_ID = process.env.WORKSPACE_ID || "";

async function apiFetch(path, opts = {}) {
  const url = path.startsWith("http") ? path : `${API_URL}${path}`;
  const headers = { ...opts.headers };
  if (API_TOKEN) headers["Authorization"] = `Bearer ${API_TOKEN}`;
  if (WORKSPACE_ID) headers["x-workspace-id"] = WORKSPACE_ID;
  if (opts.json) {
    headers["Content-Type"] = "application/json";
    opts.body = JSON.stringify(opts.json);
    delete opts.json;
  }
  const res = await fetch(url, { ...opts, headers });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`API ${res.status}: ${text.slice(0, 500)}`);
  }
  const ct = res.headers.get("content-type") || "";
  return ct.includes("application/json") ? res.json() : res.text();
}

// ── Tool definitions ────────────────────────────────────────────────────

const TOOLS = [
  // -- BorgSearch tools --
  {
    name: "search_documents",
    description:
      "Search project documents using hybrid semantic + keyword search. " +
      "Returns the most relevant document chunks with scores. " +
      "Use this FIRST when the user asks anything about their documents, contracts, filings, or any uploaded content. " +
      "With large document sets (hundreds or thousands), always search rather than trying to read files sequentially. " +
      "You can filter by doc_type (e.g. 'contract', 'filing', 'memo') and jurisdiction. " +
      "If no project corpus is attached, this returns `no_project_corpus` so you can ask for the right matter/project.",
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "Natural language search query — be specific (e.g. 'indemnification clause in vendor agreements' not just 'indemnification')",
        },
        project_id: {
          type: "number",
          description: "Project ID to search within. Defaults to current project.",
        },
        limit: {
          type: "number",
          description: "Max results (1-100, default 20). Use higher limits for broad surveys.",
        },
        doc_type: {
          type: "string",
          description: "Filter by document type (e.g. 'contract', 'filing', 'memo', 'correspondence')",
        },
        jurisdiction: {
          type: "string",
          description: "Filter by jurisdiction (e.g. 'US', 'CA', 'UK')",
        },
        exclude: {
          type: "string",
          description: "Comma-separated terms to EXCLUDE from results (NOT filter). E.g. 'indemnification,hold harmless' to find docs WITHOUT those terms.",
        },
      },
      required: ["query"],
    },
  },
  {
    name: "list_documents",
    description:
      "List all documents in a project with optional name/path filter. " +
      "Use this to get an overview of available documents, check what's been uploaded, " +
      "or find specific files by name. Supports pagination for large document sets. " +
      "If no project corpus is attached, this returns `no_project_corpus`.",
    inputSchema: {
      type: "object",
      properties: {
        project_id: {
          type: "number",
          description: "Project ID. Defaults to current project.",
        },
        filter: {
          type: "string",
          description: "Optional filename/path filter (substring match)",
        },
        limit: {
          type: "number",
          description: "Max results per page (1-200, default 50)",
        },
        offset: {
          type: "number",
          description: "Pagination offset (default 0)",
        },
      },
    },
  },
  {
    name: "read_document",
    description:
      "Read the full text content of a specific document by its file ID. " +
      "Use after search_documents to read the complete text of a relevant result. " +
      "The file ID comes from search_documents or list_documents results. " +
      "If no project corpus is attached, this returns `no_project_corpus`.",
    inputSchema: {
      type: "object",
      properties: {
        file_id: {
          type: "number",
          description: "The document file ID (from search or list results)",
        },
        project_id: {
          type: "number",
          description: "Project ID. Defaults to current project.",
        },
      },
      required: ["file_id"],
    },
  },

  // -- Pipeline task tools --
  {
    name: "create_task",
    description:
      "Create a pipeline task for long-running work that needs to run asynchronously. " +
      "Use this for: code changes, document drafting/generation, research that takes multiple steps, " +
      "any work that modifies files or needs testing. " +
      "Do NOT use for: quick questions, simple lookups, conversational replies, document searches. " +
      "Always gather enough context (ask clarifying questions) before creating a task.",
    inputSchema: {
      type: "object",
      properties: {
        title: {
          type: "string",
          description: "Short, descriptive title for the task",
        },
        description: {
          type: "string",
          description: "Detailed description with all context the pipeline agent needs. Include relevant document IDs, specific requirements, and acceptance criteria.",
        },
        project_id: {
          type: "number",
          description: "Project ID to associate the task with. Defaults to current project.",
        },
        mode: {
          type: "string",
          description: "Pipeline mode (e.g. 'lawborg', 'sweborg'). Defaults to current project mode.",
        },
        task_type: {
          type: "string",
          description: "Optional task type for categorization (e.g. 'draft', 'review', 'research')",
        },
        requires_exhaustive_corpus_review: {
          type: "boolean",
          description:
            "Set true when the task must review the full attached corpus before making corpus-wide conclusions. " +
            "Use for clause extraction, compliance audits, due diligence sweeps, or any task that needs coverage checks rather than spot retrieval.",
        },
      },
      required: ["title", "description"],
    },
  },
  {
    name: "get_task_status",
    description:
      "Check the current status of a pipeline task. " +
      "Use after creating a task to report progress, or when the user asks about task status.",
    inputSchema: {
      type: "object",
      properties: {
        task_id: {
          type: "number",
          description: "The task ID to check",
        },
      },
      required: ["task_id"],
    },
  },
  {
    name: "list_project_tasks",
    description:
      "List all pipeline tasks for a project, ordered by most recent first. " +
      "Use to show the user what work has been done, is in progress, or needs review.",
    inputSchema: {
      type: "object",
      properties: {
        project_id: {
          type: "number",
          description: "Project ID. Defaults to current project.",
        },
      },
    },
  },
  {
    name: "check_coverage",
    description:
      "COMPLETENESS CHECK: Given a search query, returns which project documents matched AND which did NOT match. " +
      "CRITICAL for exhaustive reviews — search alone only finds what exists, this finds what's MISSING. " +
      "Use after search_documents to verify you haven't missed any documents. " +
      "Example: after searching for 'indemnification clause', check_coverage tells you which contracts have NO indemnification language. " +
      "If no project corpus is attached, this returns `no_project_corpus`.",
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "Search query to check coverage against all project documents",
        },
        project_id: {
          type: "number",
          description: "Project ID. Defaults to current project.",
        },
        limit: {
          type: "number",
          description: "Max search results for matching (default 100 — use high values for coverage checks)",
        },
      },
      required: ["query"],
    },
  },
  {
    name: "get_document_categories",
    description:
      "Get all document type categories and jurisdictions in a project with counts. " +
      "Use at the START of any analysis to understand the full scope of the document corpus. " +
      "Returns faceted counts like: contract (45), filing (12), memo (8), etc. " +
      "This tells you upfront how many document types exist so you can ensure complete coverage. " +
      "If no project corpus is attached, this returns `no_project_corpus`.",
    inputSchema: {
      type: "object",
      properties: {
        project_id: {
          type: "number",
          description: "Project ID. Defaults to current project.",
        },
      },
    },
  },
  {
    name: "list_services",
    description:
      "List all available services and their status. " +
      "Call this FIRST at the start of a conversation to discover what tools and integrations are available. " +
      "Returns: current project context, available MCP servers, configured API keys, and search capabilities.",
    inputSchema: {
      type: "object",
      properties: {},
    },
  },

  // -- Knowledge base tools --
  {
    name: "upload_to_knowledge",
    description:
      "Upload a local file to the knowledge base. " +
      "Use this when the user sends a file and asks to add it to org knowledge, personal knowledge, or a project. " +
      "The file must exist at the given path on the server filesystem. " +
      "Scope: 'org' = shared org knowledge, 'user' = your personal knowledge, 'project' = attached to a project.",
    inputSchema: {
      type: "object",
      properties: {
        file_path: {
          type: "string",
          description: "Absolute path to the file on the server filesystem",
        },
        scope: {
          type: "string",
          enum: ["org", "user", "project"],
          description: "Where to store the file: org (shared), user (personal), or project",
        },
        project_id: {
          type: "number",
          description: "Project ID — required when scope is 'project'",
        },
        description: {
          type: "string",
          description: "Optional description for the file",
        },
        category: {
          type: "string",
          description: "Optional category (e.g. 'reference', 'template', 'contract')",
        },
      },
      required: ["file_path", "scope"],
    },
  },
  {
    name: "list_knowledge_files",
    description:
      "List files in the knowledge base. " +
      "Returns org-level files, your personal files, or both depending on scope.",
    inputSchema: {
      type: "object",
      properties: {
        scope: {
          type: "string",
          enum: ["org", "user", "both"],
          description: "Which knowledge to list (default: both)",
        },
      },
    },
  },
  {
    name: "list_projects",
    description:
      "List all projects/matters in the workspace. " +
      "Use to find project IDs for routing files or creating tasks.",
    inputSchema: {
      type: "object",
      properties: {},
    },
  },
];

// ── Tool handlers ───────────────────────────────────────────────────────

function resolveProjectId(args) {
  return args.project_id || (PROJECT_ID ? Number(PROJECT_ID) : null);
}

function noProjectCorpus(toolName) {
  return {
    status: "no_project_corpus",
    tool: toolName,
    current_project_id: PROJECT_ID ? Number(PROJECT_ID) : null,
    message: "No project corpus is attached to this session.",
    next_step: "Select or attach the relevant matter/project, or pass project_id explicitly.",
  };
}

async function handleSearchDocuments(args) {
  const pid = resolveProjectId(args);
  if (!pid) return noProjectCorpus("search_documents");
  const params = new URLSearchParams({ q: args.query });
  params.set("project_id", String(pid));
  if (args.limit) params.set("limit", String(args.limit));
  if (args.doc_type) params.set("doc_type", args.doc_type);
  if (args.jurisdiction) params.set("jurisdiction", args.jurisdiction);
  if (args.exclude) params.set("exclude", args.exclude);
  return apiFetch(`/api/borgsearch/query?${params}`);
}

async function handleListDocuments(args) {
  const pid = resolveProjectId(args);
  if (!pid) return noProjectCorpus("list_documents");
  const params = new URLSearchParams({ project_id: String(pid) });
  if (args.filter) params.set("q", args.filter);
  if (args.limit) params.set("limit", String(args.limit));
  if (args.offset) params.set("offset", String(args.offset));
  return apiFetch(`/api/borgsearch/files?${params}`);
}

async function handleReadDocument(args) {
  const pid = resolveProjectId(args);
  if (!pid) return noProjectCorpus("read_document");
  return apiFetch(`/api/borgsearch/file/${args.file_id}?project_id=${pid}`);
}

async function handleCreateTask(args) {
  const pid = resolveProjectId(args);
  if (!pid) return "Cannot create task: no project_id specified and no current project context.";
  const body = {
    title: args.title,
    description: args.description,
    project_id: pid,
  };
  if (args.mode) body.mode = args.mode;
  else if (PROJECT_MODE) body.mode = PROJECT_MODE;
  if (args.task_type) body.task_type = args.task_type;
  if (args.requires_exhaustive_corpus_review === true) {
    body.requires_exhaustive_corpus_review = true;
  }
  if (CHAT_THREAD) {
    body.chat_thread = CHAT_THREAD;
    body.notify_chat = CHAT_THREAD;
  }
  const result = await apiFetch("/api/tasks/create", { method: "POST", json: body });
  return `Task #${result.id} created: "${args.title}"\nThe pipeline will pick it up shortly. You'll be notified in this chat when it completes. Use get_task_status to check progress.`;
}

async function handleGetTaskStatus(args) {
  const task = await apiFetch(`/api/tasks/${args.task_id}`);
  const lines = [
    `Task #${task.id}: ${task.title}`,
    `Status: ${task.status}`,
    `Attempt: ${task.attempt}/${task.max_attempts}`,
  ];
  if (task.branch) lines.push(`Branch: ${task.branch}`);
  if (task.last_error) lines.push(`Last error: ${task.last_error}`);
  if (task.review_status) lines.push(`Review: ${task.review_status}`);
  if (task.started_at) lines.push(`Started: ${task.started_at}`);
  if (task.completed_at) lines.push(`Completed: ${task.completed_at}`);
  if (task.description) lines.push(`\nDescription: ${task.description}`);
  return lines.join("\n");
}

async function handleListProjectTasks(args) {
  const pid = resolveProjectId(args);
  if (!pid) return "No project_id specified and no current project context.";
  const tasks = await apiFetch(`/api/projects/${pid}/tasks`);
  if (!tasks.length) return "No tasks found for this project.";
  const lines = [`Project tasks (${tasks.length} total):\n`];
  for (const t of tasks.slice(0, 30)) {
    const status = t.review_status ? `${t.status} (review: ${t.review_status})` : t.status;
    lines.push(`  #${t.id} [${status}] ${t.title}`);
  }
  if (tasks.length > 30) lines.push(`  ... and ${tasks.length - 30} more`);
  return lines.join("\n");
}

async function handleListServices() {
  const lines = [];

  // Project context
  if (PROJECT_ID) {
    lines.push(`Current project: #${PROJECT_ID} (mode: ${PROJECT_MODE || "general"})`);
  } else {
    lines.push("Current project: none (global context)");
    lines.push("Project corpus: none attached. BorgSearch corpus tools remain available and will return `no_project_corpus` until project_id is provided.");
  }

  // BorgSearch
  lines.push("\n## BorgSearch (borg-mcp)");
  lines.push("  search_documents — hybrid semantic + keyword search");
  lines.push("  list_documents — browse project files");
  lines.push("  read_document — read full document text");
  lines.push("  create_task — create async pipeline tasks");
  lines.push("  get_task_status / list_project_tasks — track work");
  lines.push("\n## Knowledge Management (borg-mcp)");
  lines.push("  upload_to_knowledge — upload a local file to org/user/project knowledge");
  lines.push("  list_knowledge_files — list org or personal knowledge files");
  lines.push("  list_projects — list all projects with IDs");

  // Check which external services have API keys configured
  // These are set by the borg server when it wires MCP servers
  const externalKeys = {
    LEXISNEXIS_API_KEY: "LexisNexis (case law, statutes, legal analytics)",
    WESTLAW_API_KEY: "Westlaw (case law, secondary sources)",
    CLIO_API_KEY: "Clio (practice management, matters, contacts)",
    IMANAGE_API_KEY: "iManage (document management)",
    NETDOCUMENTS_API_KEY: "NetDocuments (cloud DMS)",
    CONGRESS_API_KEY: "Congress.gov (US federal legislation)",
    OPENSTATES_API_KEY: "OpenStates (US state legislation)",
    CANLII_API_KEY: "CanLII (Canadian case law)",
    REGULATIONS_GOV_API_KEY: "regulations.gov (federal rulemaking)",
    COURTLISTENER_TOKEN: "CourtListener (US case law — free, enhanced with token)",
    PLAID_CLIENT_ID: "Plaid (banking, financial accounts)",
    SHOVELS_API_KEY: "Shovels (building permits, contractors)",
  };

  const configured = [];
  const free = [];
  for (const [key, label] of Object.entries(externalKeys)) {
    if (process.env[key]) configured.push(`  ${label}`);
  }

  // Free services (always available via lawborg-mcp when legal mode is active)
  const isLegal = PROJECT_MODE === "lawborg" || PROJECT_MODE === "legal";
  if (isLegal) {
    lines.push("\n## Legal Research (lawborg-mcp)");
    lines.push("Free (no key required):");
    lines.push("  CourtListener — US case law, PACER dockets");
    lines.push("  EDGAR/SEC — company filings, full-text search");
    lines.push("  Federal Register — proposed/final rules");
    lines.push("  UK Legislation — Acts, SIs");
    lines.push("  EUR-Lex — EU law");
    lines.push("  USPTO — patent data");
    if (configured.length > 0) {
      lines.push("Configured (API key present):");
      lines.push(...configured);
    }
  } else if (configured.length > 0) {
    lines.push("\n## External Services");
    lines.push("Configured:");
    lines.push(...configured);
  }

  if (!isLegal && configured.length === 0) {
    lines.push("\n## External Services");
    lines.push("  No external API keys configured for this session.");
  }

  return lines.join("\n");
}

async function handleGetDocumentCategories(args) {
  const pid = resolveProjectId(args);
  if (!pid) return noProjectCorpus("get_document_categories");
  const data = await apiFetch(`/api/borgsearch/facets?project_id=${pid}`);
  const lines = [`Document categories for project #${pid}:\n`];

  if (data.doc_types?.length > 0) {
    lines.push("## Document Types");
    for (const dt of data.doc_types) {
      lines.push(`  ${dt.value}: ${dt.count} documents`);
    }
    lines.push("");
  }

  if (data.jurisdictions?.length > 0) {
    lines.push("## Jurisdictions");
    for (const j of data.jurisdictions) {
      lines.push(`  ${j.value}: ${j.count} documents`);
    }
  }

  if (!data.doc_types?.length && !data.jurisdictions?.length) {
    lines.push("No categorized documents found. Documents may not be indexed yet.");
  }

  return lines.join("\n");
}

async function handleCheckCoverage(args) {
  const pid = resolveProjectId(args);
  if (!pid) return noProjectCorpus("check_coverage");
  const limit = args.limit || 100;
  const params = new URLSearchParams({ q: args.query, project_id: String(pid), limit: String(limit) });
  return apiFetch(`/api/borgsearch/coverage?${params}`);
}

async function handleUploadToKnowledge(args) {
  const { file_path, scope, project_id, description, category } = args;
  if (!file_path) return "file_path is required";
  if (!scope) return "scope is required";

  // Read the file
  const { readFile } = await import("fs/promises");
  let fileBytes;
  try {
    fileBytes = await readFile(file_path);
  } catch (e) {
    return `Cannot read file at ${file_path}: ${e.message}`;
  }

  const filename = file_path.split("/").pop();
  const FormData = (await import("form-data")).default;
  const form = new FormData();
  form.append("file", fileBytes, { filename });
  if (description) form.append("description", description);
  if (category) form.append("category", category);

  let endpoint;
  if (scope === "user") {
    endpoint = "/api/knowledge/my/upload";
  } else if (scope === "project") {
    const pid = project_id || (PROJECT_ID ? Number(PROJECT_ID) : null);
    if (!pid) return "project_id is required for scope=project";
    endpoint = `/api/projects/${pid}/files/upload`;
  } else {
    endpoint = "/api/knowledge/upload";
  }

  const url = `${API_URL}${endpoint}`;
  const headers = { ...form.getHeaders() };
  if (API_TOKEN) headers["Authorization"] = `Bearer ${API_TOKEN}`;

  const res = await fetch(url, { method: "POST", headers, body: form.getBuffer() });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    return `Upload failed (${res.status}): ${text.slice(0, 300)}`;
  }
  const result = await res.json().catch(() => ({}));
  return `File "${filename}" uploaded successfully to ${scope} knowledge. ID: ${result.id || "?"}`;
}

async function handleListKnowledgeFiles(args) {
  const scope = args.scope || "both";
  const lines = [];

  if (scope === "org" || scope === "both") {
    const data = await apiFetch("/api/knowledge?limit=50").catch((e) => ({ files: [], error: e.message }));
    if (data.files?.length > 0) {
      lines.push(`## Org Knowledge (${data.total || data.files.length} files)`);
      for (const f of data.files.slice(0, 30)) {
        lines.push(`  [${f.id}] ${f.file_name} — ${f.description || "(no description)"}`);
      }
    } else {
      lines.push("## Org Knowledge: (empty)");
    }
  }

  if (scope === "user" || scope === "both") {
    const data = await apiFetch("/api/knowledge/my?limit=50").catch((e) => ({ files: [], error: e.message }));
    if (data.files?.length > 0) {
      lines.push(`\n## My Knowledge (${data.total || data.files.length} files)`);
      for (const f of data.files.slice(0, 30)) {
        lines.push(`  [${f.id}] ${f.file_name} — ${f.description || "(no description)"}`);
      }
    } else {
      lines.push("\n## My Knowledge: (empty)");
    }
  }

  return lines.join("\n") || "No knowledge files found.";
}

async function handleListProjects() {
  const data = await apiFetch("/api/projects?limit=100");
  if (!data?.length && !Array.isArray(data)) return "No projects found.";
  const projects = Array.isArray(data) ? data : (data.projects || []);
  if (!projects.length) return "No projects found.";
  const lines = [`Projects (${projects.length}):\n`];
  for (const p of projects.slice(0, 50)) {
    lines.push(`  #${p.id} [${p.mode || "general"}] ${p.name}`);
  }
  return lines.join("\n");
}

const HANDLERS = {
  search_documents: handleSearchDocuments,
  list_documents: handleListDocuments,
  read_document: handleReadDocument,
  check_coverage: handleCheckCoverage,
  get_document_categories: handleGetDocumentCategories,
  create_task: handleCreateTask,
  get_task_status: handleGetTaskStatus,
  list_project_tasks: handleListProjectTasks,
  list_services: handleListServices,
  upload_to_knowledge: handleUploadToKnowledge,
  list_knowledge_files: handleListKnowledgeFiles,
  list_projects: handleListProjects,
};

// ── MCP server setup ────────────────────────────────────────────────────

const server = new Server(
  { name: "borg", version: "0.1.0" },
  { capabilities: { tools: {} } },
);

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: TOOLS,
}));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args = {} } = request.params;
  const handler = HANDLERS[name];
  if (!handler) {
    return {
      content: [{ type: "text", text: `Unknown tool: ${name}` }],
      isError: true,
    };
  }
  try {
    const result = await handler(args);
    const text = typeof result === "string" ? result : JSON.stringify(result, null, 2);
    return { content: [{ type: "text", text }] };
  } catch (err) {
    return {
      content: [{ type: "text", text: `Error: ${err.message}` }],
      isError: true,
    };
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
