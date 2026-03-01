#!/usr/bin/env node
// LexisNexis MCP server — exposes all LN APIs as tools for Claude agents.
// Reads API key from LEXISNEXIS_API_KEY env var.

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

const API_KEY = process.env.LEXISNEXIS_API_KEY || "";
const LEXIS_BASE = process.env.LEXIS_BASE_URL || "https://api.lexisnexis.com/v1";
const STATENET_BASE = process.env.STATENET_BASE_URL || "https://api.lexisnexis.com/statenet/v1";
const LEXMACHINA_BASE = process.env.LEXMACHINA_BASE_URL || "https://api.lexmachina.com/v1";
const INTELLIGIZE_BASE = process.env.INTELLIGIZE_BASE_URL || "https://api.intelligize.com/v1";
const COGNITIVE_BASE = process.env.COGNITIVE_BASE_URL || "https://api.lexisnexis.com/cognitive/v1";

async function apiCall(base, path, method = "GET", body = null) {
  const url = `${base}${path}`;
  const opts = {
    method,
    headers: {
      Authorization: `Bearer ${API_KEY}`,
      "Content-Type": "application/json",
    },
  };
  if (body) opts.body = JSON.stringify(body);
  const resp = await fetch(url, opts);
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(`${resp.status} ${resp.statusText}: ${text}`);
  }
  return resp.json();
}

// Tool definitions
const TOOLS = [
  // ── Lexis API ──────────────────────────────────────────────────────
  {
    name: "lexis_search",
    description:
      "Search LexisNexis for case law, secondary sources, and legal content. " +
      "Returns matching documents with citations and summaries.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query (natural language or Boolean)" },
        jurisdiction: { type: "string", description: "Jurisdiction filter (e.g. 'US', 'CA', 'NY')" },
        date_from: { type: "string", description: "Start date (YYYY-MM-DD)" },
        date_to: { type: "string", description: "End date (YYYY-MM-DD)" },
        content_type: { type: "string", description: "Content type: cases, statutes, secondary, all" },
        limit: { type: "number", description: "Max results (default 20)" },
      },
      required: ["query"],
    },
  },
  {
    name: "lexis_retrieve",
    description: "Retrieve the full text of a document from LexisNexis by document ID.",
    inputSchema: {
      type: "object",
      properties: {
        document_id: { type: "string", description: "LexisNexis document ID" },
      },
      required: ["document_id"],
    },
  },
  {
    name: "lexis_shepards",
    description:
      "Check Shepard's citation treatment for a legal citation. " +
      "Shows whether the case has been affirmed, distinguished, overruled, etc.",
    inputSchema: {
      type: "object",
      properties: {
        citation: { type: "string", description: "Legal citation (e.g. '410 U.S. 113')" },
      },
      required: ["citation"],
    },
  },
  // ── State Net API ──────────────────────────────────────────────────
  {
    name: "statenet_search_bills",
    description: "Search for bills and legislation by keyword, state, and session.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        state: { type: "string", description: "State abbreviation (e.g. 'CA', 'NY')" },
        session: { type: "string", description: "Legislative session" },
        status: { type: "string", description: "Bill status filter" },
        limit: { type: "number", description: "Max results" },
      },
      required: ["query"],
    },
  },
  {
    name: "statenet_get_bill",
    description: "Retrieve full bill details including text, history, and sponsors.",
    inputSchema: {
      type: "object",
      properties: {
        bill_id: { type: "string", description: "Bill identifier" },
      },
      required: ["bill_id"],
    },
  },
  {
    name: "statenet_search_regulations",
    description: "Search federal register and state regulations.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        agency: { type: "string", description: "Issuing agency" },
        date_from: { type: "string", description: "Start date" },
        date_to: { type: "string", description: "End date" },
        limit: { type: "number", description: "Max results" },
      },
      required: ["query"],
    },
  },
  {
    name: "statenet_get_statute",
    description: "Retrieve statute text by citation.",
    inputSchema: {
      type: "object",
      properties: {
        citation: { type: "string", description: "Statute citation" },
      },
      required: ["citation"],
    },
  },
  // ── Lex Machina API ────────────────────────────────────────────────
  {
    name: "lexmachina_search_cases",
    description:
      "Search litigation analytics — find cases by party, attorney, judge, court, or case type. " +
      "Returns case resolutions, damages, and timing data.",
    inputSchema: {
      type: "object",
      properties: {
        party: { type: "string", description: "Party name" },
        attorney: { type: "string", description: "Attorney name" },
        judge: { type: "string", description: "Judge name" },
        court: { type: "string", description: "Court name or abbreviation" },
        case_type: { type: "string", description: "Case type (e.g. patent, antitrust)" },
        date_from: { type: "string", description: "Filed after date" },
        date_to: { type: "string", description: "Filed before date" },
        limit: { type: "number", description: "Max results" },
      },
    },
  },
  {
    name: "lexmachina_case_details",
    description: "Get full case analytics: resolutions, damages awarded, remedies, and timing.",
    inputSchema: {
      type: "object",
      properties: {
        case_id: { type: "string", description: "Lex Machina case ID" },
      },
      required: ["case_id"],
    },
  },
  {
    name: "lexmachina_judge_profile",
    description: "Get judge analytics: ruling patterns, case duration, and outcomes by case type.",
    inputSchema: {
      type: "object",
      properties: {
        judge_id: { type: "string", description: "Judge identifier" },
      },
      required: ["judge_id"],
    },
  },
  {
    name: "lexmachina_party_history",
    description: "Get a party's litigation history: cases filed, win rates, typical damages.",
    inputSchema: {
      type: "object",
      properties: {
        party_name: { type: "string", description: "Party name to look up" },
      },
      required: ["party_name"],
    },
  },
  // ── Intelligize API ────────────────────────────────────────────────
  {
    name: "intelligize_search_filings",
    description: "Search SEC filings (10-K, 10-Q, 8-K, proxy) by company and type.",
    inputSchema: {
      type: "object",
      properties: {
        company: { type: "string", description: "Company name or ticker" },
        filing_type: { type: "string", description: "Filing type (10-K, 10-Q, 8-K, proxy)" },
        date_from: { type: "string", description: "Start date" },
        date_to: { type: "string", description: "End date" },
        limit: { type: "number", description: "Max results" },
      },
    },
  },
  {
    name: "intelligize_get_filing",
    description: "Retrieve an SEC filing by ID, optionally a specific section.",
    inputSchema: {
      type: "object",
      properties: {
        filing_id: { type: "string", description: "Filing identifier" },
        section: { type: "string", description: "Specific section to retrieve" },
      },
      required: ["filing_id"],
    },
  },
  {
    name: "intelligize_search_clauses",
    description: "Find specific clause language across SEC filings.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Clause text to search for" },
        filing_type: { type: "string", description: "Limit to filing type" },
        limit: { type: "number", description: "Max results" },
      },
      required: ["query"],
    },
  },
  // ── Cognitive APIs ─────────────────────────────────────────────────
  {
    name: "cognitive_resolve_judge",
    description: "Resolve a judge name to a canonical entity with court assignments and metadata.",
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: "Judge name to resolve" },
      },
      required: ["name"],
    },
  },
  {
    name: "cognitive_resolve_court",
    description: "Resolve a court name or abbreviation to a canonical entity.",
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: "Court name or abbreviation" },
      },
      required: ["name"],
    },
  },
  {
    name: "cognitive_legal_define",
    description: "Look up a legal term definition with context and related terms.",
    inputSchema: {
      type: "object",
      properties: {
        term: { type: "string", description: "Legal term to define" },
      },
      required: ["term"],
    },
  },
  {
    name: "cognitive_redact_pii",
    description: "Detect and redact personally identifiable information from text.",
    inputSchema: {
      type: "object",
      properties: {
        text: { type: "string", description: "Text to redact PII from" },
      },
      required: ["text"],
    },
  },
  {
    name: "cognitive_translate",
    description: "Translate legal text to a target language.",
    inputSchema: {
      type: "object",
      properties: {
        text: { type: "string", description: "Text to translate" },
        target_language: { type: "string", description: "Target language code (e.g. 'es', 'fr', 'de')" },
      },
      required: ["text", "target_language"],
    },
  },
];

// Tool handlers
async function handleTool(name, args) {
  switch (name) {
    // Lexis
    case "lexis_search":
      return apiCall(LEXIS_BASE, "/search", "POST", args);
    case "lexis_retrieve":
      return apiCall(LEXIS_BASE, `/documents/${args.document_id}`);
    case "lexis_shepards":
      return apiCall(LEXIS_BASE, `/shepards/${encodeURIComponent(args.citation)}`);

    // State Net
    case "statenet_search_bills":
      return apiCall(STATENET_BASE, "/bills/search", "POST", args);
    case "statenet_get_bill":
      return apiCall(STATENET_BASE, `/bills/${args.bill_id}`);
    case "statenet_search_regulations":
      return apiCall(STATENET_BASE, "/regulations/search", "POST", args);
    case "statenet_get_statute":
      return apiCall(STATENET_BASE, `/statutes/${encodeURIComponent(args.citation)}`);

    // Lex Machina
    case "lexmachina_search_cases":
      return apiCall(LEXMACHINA_BASE, "/cases/search", "POST", args);
    case "lexmachina_case_details":
      return apiCall(LEXMACHINA_BASE, `/cases/${args.case_id}`);
    case "lexmachina_judge_profile":
      return apiCall(LEXMACHINA_BASE, `/judges/${args.judge_id}`);
    case "lexmachina_party_history": {
      const qs = new URLSearchParams({ name: args.party_name });
      return apiCall(LEXMACHINA_BASE, `/parties?${qs}`);
    }

    // Intelligize
    case "intelligize_search_filings":
      return apiCall(INTELLIGIZE_BASE, "/filings/search", "POST", args);
    case "intelligize_get_filing": {
      const path = args.section
        ? `/filings/${args.filing_id}?section=${encodeURIComponent(args.section)}`
        : `/filings/${args.filing_id}`;
      return apiCall(INTELLIGIZE_BASE, path);
    }
    case "intelligize_search_clauses":
      return apiCall(INTELLIGIZE_BASE, "/clauses/search", "POST", args);

    // Cognitive
    case "cognitive_resolve_judge": {
      const qs = new URLSearchParams({ name: args.name });
      return apiCall(COGNITIVE_BASE, `/entities/judges?${qs}`);
    }
    case "cognitive_resolve_court": {
      const qs = new URLSearchParams({ name: args.name });
      return apiCall(COGNITIVE_BASE, `/entities/courts?${qs}`);
    }
    case "cognitive_legal_define":
      return apiCall(COGNITIVE_BASE, `/dictionary/${encodeURIComponent(args.term)}`);
    case "cognitive_redact_pii":
      return apiCall(COGNITIVE_BASE, "/redact", "POST", { text: args.text });
    case "cognitive_translate":
      return apiCall(COGNITIVE_BASE, "/translate", "POST", {
        text: args.text,
        target_language: args.target_language,
      });

    default:
      throw new Error(`Unknown tool: ${name}`);
  }
}

// MCP server setup
const server = new Server(
  { name: "lexis-mcp", version: "0.1.0" },
  { capabilities: { tools: {} } },
);

server.setRequestHandler("tools/list", async () => ({ tools: TOOLS }));

server.setRequestHandler("tools/call", async (request) => {
  const { name, arguments: args } = request.params;
  if (!API_KEY) {
    return {
      content: [
        {
          type: "text",
          text: "Error: LEXISNEXIS_API_KEY not set. Configure your API key in the dashboard under Settings > API Keys.",
        },
      ],
      isError: true,
    };
  }
  try {
    const result = await handleTool(name, args || {});
    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  } catch (err) {
    return {
      content: [{ type: "text", text: `Error: ${err.message}` }],
      isError: true,
    };
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
