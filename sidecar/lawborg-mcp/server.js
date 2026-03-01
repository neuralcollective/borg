#!/usr/bin/env node
// Unified legal MCP server — exposes free public APIs unconditionally
// and BYOK provider tools when API keys are present.

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

// ── BYOK keys (optional) ──────────────────────────────────────────────
const LEXIS_KEY = process.env.LEXISNEXIS_API_KEY || "";
const WESTLAW_KEY = process.env.WESTLAW_API_KEY || "";
const CLIO_KEY = process.env.CLIO_API_KEY || "";
const IMANAGE_KEY = process.env.IMANAGE_API_KEY || "";
const NETDOCUMENTS_KEY = process.env.NETDOCUMENTS_API_KEY || "";
const CONGRESS_KEY = process.env.CONGRESS_API_KEY || "";
const OPENSTATES_KEY = process.env.OPENSTATES_API_KEY || "";
const CANLII_KEY = process.env.CANLII_API_KEY || "";
const REGULATIONS_KEY = process.env.REGULATIONS_GOV_API_KEY || "";

// ── Base URLs ──────────────────────────────────────────────────────────
const COURTLISTENER = "https://www.courtlistener.com/api/rest/v4";
const EDGAR = "https://efts.sec.gov/LATEST";
const EDGAR_DATA = "https://data.sec.gov";
const FED_REGISTER = "https://www.federalregister.gov/api/v1";
const REGULATIONS = "https://api.regulations.gov/v4";
const CONGRESS = "https://api.congress.gov/v3";
const UK_LEG = "https://www.legislation.gov.uk";
const EURLEX = "https://eur-lex.europa.eu/eurlex-ws/rest";
const OPENSTATES = "https://v3.openstates.org";
const CANLII_BASE = "https://api.canlii.org/v1";
const USPTO = "https://developer.uspto.gov/ibd-api/v1";

const LEXIS_BASE = process.env.LEXIS_BASE_URL || "https://api.lexisnexis.com/v1";
const STATENET_BASE = process.env.STATENET_BASE_URL || "https://api.lexisnexis.com/statenet/v1";
const LEXMACHINA_BASE = process.env.LEXMACHINA_BASE_URL || "https://api.lexmachina.com/v1";
const INTELLIGIZE_BASE = process.env.INTELLIGIZE_BASE_URL || "https://api.intelligize.com/v1";
const COGNITIVE_BASE = process.env.COGNITIVE_BASE_URL || "https://api.lexisnexis.com/cognitive/v1";
const WESTLAW_BASE = process.env.WESTLAW_BASE_URL || "https://api.thomsonreuters.com/legal/v1";
const CLIO_BASE = process.env.CLIO_BASE_URL || "https://app.clio.com/api/v4";
const IMANAGE_BASE = process.env.IMANAGE_BASE_URL || "https://cloudimanage.com/work/api/v2";
const NETDOCS_BASE = process.env.NETDOCUMENTS_BASE_URL || "https://api.vault.netvoyage.com/v2";

// ── Rate limiting ────────────────────────────────────────────────────
class RateLimiter {
  constructor(maxRequests, windowMs) {
    this.max = maxRequests;
    this.window = windowMs;
    this.timestamps = [];
  }
  check(label) {
    const now = Date.now();
    this.timestamps = this.timestamps.filter(t => now - t < this.window);
    if (this.timestamps.length >= this.max) {
      throw new Error(`Rate limit exceeded for ${label} (${this.max} requests per ${this.window / 1000}s)`);
    }
    this.timestamps.push(now);
  }
}

const rateLimiters = [
  { test: url => url.includes("courtlistener.com"),  limiter: new RateLimiter(5000, 3600000), label: "CourtListener" },
  { test: url => url.includes("sec.gov"),             limiter: new RateLimiter(10, 1000),      label: "EDGAR/SEC" },
  { test: url => url.includes("regulations.gov"),     limiter: new RateLimiter(500, 3600000),   label: "regulations.gov" },
  { test: url => url.includes("congress.gov"),         limiter: new RateLimiter(1000, 3600000),  label: "Congress.gov" },
  { test: url => url.includes("canlii.org"),           limiter: new RateLimiter(300, 3600000),   label: "CanLII" },
  { test: url => url.includes("openstates.org"),       limiter: new RateLimiter(600, 3600000),   label: "Open States" },
];

function checkRateLimit(url) {
  for (const { test, limiter, label } of rateLimiters) {
    if (test(url)) { limiter.check(label); return; }
  }
}

// ── Simple LRU cache ───────────────────────────────────────────────────
const cache = new Map();
const CACHE_TTL = 10 * 60 * 1000; // 10 min
const MAX_CACHE = 200;

function cacheKey(prefix, args) {
  return prefix + ":" + JSON.stringify(args);
}

function cached(key) {
  const entry = cache.get(key);
  if (entry && Date.now() - entry.ts < CACHE_TTL) return entry.val;
  if (entry) cache.delete(key);
  return null;
}

function setCache(key, val) {
  if (cache.size >= MAX_CACHE) {
    const oldest = cache.keys().next().value;
    cache.delete(oldest);
  }
  cache.set(key, { val, ts: Date.now() });
  return val;
}

// ── HTTP helpers ───────────────────────────────────────────────────────
const UA = "LegalMCP/0.2 (borg-legal-agent; contact@neuralcollective.ai)";

async function fetchJSON(url, opts = {}) {
  checkRateLimit(url);
  const headers = { "User-Agent": UA, Accept: "application/json", ...opts.headers };
  const resp = await fetch(url, { ...opts, headers });
  if (!resp.ok) {
    const text = await resp.text().catch(() => "");
    throw new Error(`${resp.status} ${resp.statusText}: ${text.slice(0, 500)}`);
  }
  const ct = resp.headers.get("content-type") || "";
  if (ct.includes("json")) return resp.json();
  return { text: await resp.text() };
}

async function authedCall(base, path, key, method = "GET", body = null) {
  const url = `${base}${path}`;
  const opts = {
    method,
    headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
  };
  if (body) opts.body = JSON.stringify(body);
  return fetchJSON(url, opts);
}

function qs(params) {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== "") p.set(k, String(v));
  }
  return p.toString();
}

function requireKey(name, key, signupUrl) {
  if (!key) {
    const msg = signupUrl
      ? `${name} API key not configured. Get a free key at ${signupUrl}`
      : `${name} API key not configured. Add it in the dashboard under Settings > API Keys.`;
    throw new Error(msg);
  }
}

function buildParams(args, fields) {
  const params = {};
  for (const f of fields) {
    const [src, dst] = Array.isArray(f) ? f : [f, f];
    if (args[src] !== undefined && args[src] !== null) params[dst] = args[src];
  }
  return params;
}

function validateId(id) {
  if (!/^[a-zA-Z0-9._-]+$/.test(String(id))) throw new Error(`Invalid ID: ${id}`);
  return id;
}

// ═══════════════════════════════════════════════════════════════════════
// TOOL DEFINITIONS
// ═══════════════════════════════════════════════════════════════════════

// ── CourtListener / RECAP (free) ─────────────────────────────────────
const COURTLISTENER_TOOLS = [
  {
    name: "courtlistener_search_opinions",
    description: "Search US case law opinions on CourtListener. Returns citations, court, dates, and snippets. Covers federal and state courts.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "Search query (natural language or Boolean)" },
        court: { type: "string", description: "Court ID filter (e.g. 'scotus', 'ca9', 'nyed')" },
        filed_after: { type: "string", description: "Filed after date (YYYY-MM-DD)" },
        filed_before: { type: "string", description: "Filed before date (YYYY-MM-DD)" },
        cited_gt: { type: "number", description: "Minimum citation count" },
        ordering: { type: "string", description: "Sort: 'score desc', 'dateFiled desc', 'citeCount desc'" },
        page: { type: "number", description: "Page number (default 1)" },
      },
      required: ["q"],
    },
  },
  {
    name: "courtlistener_get_opinion",
    description: "Get the full text and metadata of a court opinion by its CourtListener ID.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "number", description: "CourtListener opinion cluster ID" } },
      required: ["id"],
    },
  },
  {
    name: "courtlistener_search_dockets",
    description: "Search federal court dockets in the RECAP archive. Returns case name, court, parties, and docket number.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "Search query" },
        court: { type: "string", description: "Court ID filter" },
        filed_after: { type: "string", description: "Filed after (YYYY-MM-DD)" },
        filed_before: { type: "string", description: "Filed before (YYYY-MM-DD)" },
        nature_of_suit: { type: "string", description: "Nature of suit code" },
        page: { type: "number", description: "Page number" },
      },
      required: ["q"],
    },
  },
  {
    name: "courtlistener_get_docket",
    description: "Get full docket details including entries, parties, and attorneys by docket ID.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "number", description: "CourtListener docket ID" } },
      required: ["id"],
    },
  },
  {
    name: "courtlistener_search_judges",
    description: "Search for federal and state judges. Returns biographical info, court assignments, and appointing president.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "Judge name or search query" },
        court: { type: "string", description: "Court ID filter" },
        page: { type: "number", description: "Page number" },
      },
      required: ["q"],
    },
  },
  {
    name: "courtlistener_get_judge",
    description: "Get full judge profile by person ID — positions, education, political affiliation, appointing president.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "number", description: "CourtListener person ID" } },
      required: ["id"],
    },
  },
  {
    name: "courtlistener_search_oral_arguments",
    description: "Search oral argument audio recordings. Covers Supreme Court and many circuit courts.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "Search query" },
        court: { type: "string", description: "Court ID filter" },
        argued_after: { type: "string", description: "Argued after (YYYY-MM-DD)" },
        argued_before: { type: "string", description: "Argued before (YYYY-MM-DD)" },
        page: { type: "number", description: "Page number" },
      },
      required: ["q"],
    },
  },
  {
    name: "courtlistener_search_recap_documents",
    description: "Search PACER documents in the RECAP archive. Returns document descriptions, attachment info, and availability.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "Full-text search within documents" },
        docket_id: { type: "number", description: "Limit to specific docket" },
        description: { type: "string", description: "Filter by docket entry description" },
        page: { type: "number", description: "Page number" },
      },
      required: ["q"],
    },
  },
  {
    name: "courtlistener_citation_lookup",
    description: "Look up a case by its legal citation (e.g. '410 U.S. 113'). Returns the matching opinion.",
    inputSchema: {
      type: "object",
      properties: {
        cite: { type: "string", description: "Legal citation (e.g. '410 U.S. 113', '347 U.S. 483')" },
      },
      required: ["cite"],
    },
  },
];

// ── SEC EDGAR (free, no key) ─────────────────────────────────────────
const EDGAR_TOOLS = [
  {
    name: "edgar_fulltext_search",
    description: "Full-text search across all SEC EDGAR filings. Returns filing metadata with highlighted snippets.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "Search query" },
        dateRange: { type: "string", description: "Date range: 'custom' with startdt/enddt, or preset" },
        startdt: { type: "string", description: "Start date (YYYY-MM-DD)" },
        enddt: { type: "string", description: "End date (YYYY-MM-DD)" },
        forms: { type: "string", description: "Comma-separated form types (e.g. '10-K,10-Q,8-K')" },
        from: { type: "number", description: "Result offset (default 0)" },
      },
      required: ["q"],
    },
  },
  {
    name: "edgar_company_filings",
    description: "Get recent filings for a specific company by CIK number or ticker symbol.",
    inputSchema: {
      type: "object",
      properties: {
        cik: { type: "string", description: "CIK number (zero-padded to 10 digits) or ticker symbol" },
        type: { type: "string", description: "Filing type filter (e.g. '10-K', '10-Q')" },
        count: { type: "number", description: "Number of filings to return (default 20)" },
      },
      required: ["cik"],
    },
  },
  {
    name: "edgar_company_facts",
    description: "Get XBRL company facts — structured financial data extracted from filings. Great for financial analysis.",
    inputSchema: {
      type: "object",
      properties: {
        cik: { type: "string", description: "CIK number (zero-padded to 10 digits)" },
      },
      required: ["cik"],
    },
  },
  {
    name: "edgar_company_concept",
    description: "Get a single XBRL concept's history for a company (e.g. Revenue, Assets). Returns all reported values across filings.",
    inputSchema: {
      type: "object",
      properties: {
        cik: { type: "string", description: "CIK number (zero-padded to 10 digits)" },
        taxonomy: { type: "string", description: "Taxonomy (usually 'us-gaap' or 'dei')" },
        concept: { type: "string", description: "Concept name (e.g. 'Revenue', 'Assets', 'NetIncomeLoss')" },
      },
      required: ["cik", "taxonomy", "concept"],
    },
  },
  {
    name: "edgar_resolve_ticker",
    description: "Resolve a company name or ticker symbol to a CIK number for use with other EDGAR tools.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Company name or ticker symbol" },
      },
      required: ["query"],
    },
  },
];

// ── Federal Register (free, no key) ──────────────────────────────────
const FED_REGISTER_TOOLS = [
  {
    name: "federal_register_search",
    description: "Search the Federal Register for rules, proposed rules, notices, and presidential documents.",
    inputSchema: {
      type: "object",
      properties: {
        conditions: {
          type: "object",
          description: "Search conditions object",
          properties: {
            term: { type: "string", description: "Search term" },
            agencies: { type: "array", items: { type: "string" }, description: "Agency slug filter" },
            type: { type: "array", items: { type: "string" }, description: "Document types: RULE, PRORULE, NOTICE, PRESDOCU" },
            publication_date: { type: "object", properties: { gte: { type: "string" }, lte: { type: "string" } } },
          },
        },
        page: { type: "number", description: "Page number" },
        per_page: { type: "number", description: "Results per page (max 1000)" },
        order: { type: "string", description: "Sort: 'relevance' or 'newest'" },
      },
      required: ["conditions"],
    },
  },
  {
    name: "federal_register_get_document",
    description: "Get a Federal Register document by its document number. Returns full text, agencies, CFR references.",
    inputSchema: {
      type: "object",
      properties: {
        document_number: { type: "string", description: "Federal Register document number (e.g. '2024-12345')" },
      },
      required: ["document_number"],
    },
  },
  {
    name: "federal_register_get_agency",
    description: "Get information about a federal agency including recent documents and child agencies.",
    inputSchema: {
      type: "object",
      properties: {
        slug: { type: "string", description: "Agency slug (e.g. 'environmental-protection-agency', 'securities-and-exchange-commission')" },
      },
      required: ["slug"],
    },
  },
];

// ── regulations.gov (free API key) ───────────────────────────────────
const REGULATIONS_TOOLS = [
  {
    name: "regulations_search_documents",
    description: "Search regulations.gov for regulatory documents — rules, proposed rules, notices, and supporting materials.",
    inputSchema: {
      type: "object",
      properties: {
        filter: {
          type: "object",
          properties: {
            searchTerm: { type: "string", description: "Search term" },
            agencyId: { type: "string", description: "Agency ID (e.g. 'EPA', 'SEC', 'FDA')" },
            documentType: { type: "string", description: "Type: Rule, Proposed Rule, Notice, Other" },
            postedDate: { type: "string", description: "Date filter (YYYY-MM-DD)" },
          },
        },
        page: { type: "number" },
        pageSize: { type: "number" },
      },
      required: ["filter"],
    },
  },
  {
    name: "regulations_get_document",
    description: "Get full details of a regulatory document by its ID.",
    inputSchema: {
      type: "object",
      properties: {
        documentId: { type: "string", description: "Document ID from regulations.gov" },
      },
      required: ["documentId"],
    },
  },
  {
    name: "regulations_search_dockets",
    description: "Search regulatory dockets — each docket contains all documents for a rulemaking action.",
    inputSchema: {
      type: "object",
      properties: {
        filter: {
          type: "object",
          properties: {
            searchTerm: { type: "string", description: "Search term" },
            agencyId: { type: "string", description: "Agency ID" },
            docketType: { type: "string", description: "Type: Rulemaking, Nonrulemaking" },
          },
        },
        page: { type: "number" },
      },
      required: ["filter"],
    },
  },
  {
    name: "regulations_get_comments",
    description: "Get public comments submitted on a regulatory document.",
    inputSchema: {
      type: "object",
      properties: {
        filter: {
          type: "object",
          properties: {
            commentOnId: { type: "string", description: "Document ID that comments are on" },
            searchTerm: { type: "string", description: "Search within comments" },
          },
        },
        page: { type: "number" },
        pageSize: { type: "number" },
      },
      required: ["filter"],
    },
  },
];

// ── Congress.gov (free API key) ──────────────────────────────────────
const CONGRESS_TOOLS = [
  {
    name: "congress_search_bills",
    description: "Search bills in the US Congress. Returns bill number, title, sponsors, status, and actions.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        congress: { type: "number", description: "Congress number (e.g. 118 for 2023-2024)" },
        type: { type: "string", description: "Bill type: hr, s, hjres, sjres, hconres, sconres, hres, sres" },
        offset: { type: "number" },
        limit: { type: "number", description: "Max results (default 20)" },
      },
      required: ["query"],
    },
  },
  {
    name: "congress_get_bill",
    description: "Get full details of a specific bill including text, actions, cosponsors, and related bills.",
    inputSchema: {
      type: "object",
      properties: {
        congress: { type: "number", description: "Congress number" },
        type: { type: "string", description: "Bill type (hr, s, etc.)" },
        number: { type: "number", description: "Bill number" },
      },
      required: ["congress", "type", "number"],
    },
  },
  {
    name: "congress_get_bill_text",
    description: "Get the text versions of a bill.",
    inputSchema: {
      type: "object",
      properties: {
        congress: { type: "number", description: "Congress number" },
        type: { type: "string", description: "Bill type" },
        number: { type: "number", description: "Bill number" },
      },
      required: ["congress", "type", "number"],
    },
  },
  {
    name: "congress_search_members",
    description: "Search for current and past members of Congress.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Member name search" },
        currentMember: { type: "boolean", description: "Only current members" },
        offset: { type: "number" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "congress_get_member",
    description: "Get full details of a member of Congress by bioguide ID.",
    inputSchema: {
      type: "object",
      properties: {
        bioguideId: { type: "string", description: "Bioguide ID (e.g. 'W000817')" },
      },
      required: ["bioguideId"],
    },
  },
];

// ── UK Legislation (free, no key) ────────────────────────────────────
const UK_LEG_TOOLS = [
  {
    name: "uk_legislation_search",
    description: "Search UK legislation — Acts of Parliament, Statutory Instruments, and more. Returns titles, years, and types.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search term" },
        type: { type: "string", description: "Legislation type: ukpga (Act), uksi (Statutory Instrument), asp (Scottish Act), etc." },
        year: { type: "number", description: "Year of enactment" },
        page: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "uk_legislation_get",
    description: "Get the full text of a UK statute or SI. Returns structured XML/HTML of the legislation.",
    inputSchema: {
      type: "object",
      properties: {
        type: { type: "string", description: "Legislation type (ukpga, uksi, asp, etc.)" },
        year: { type: "number", description: "Year" },
        number: { type: "number", description: "Chapter/SI number" },
        section: { type: "string", description: "Specific section (e.g. 'section/1', 'part/2')" },
      },
      required: ["type", "year", "number"],
    },
  },
  {
    name: "uk_legislation_changes",
    description: "Get amendments and changes affecting a piece of UK legislation.",
    inputSchema: {
      type: "object",
      properties: {
        type: { type: "string", description: "Legislation type" },
        year: { type: "number", description: "Year" },
        number: { type: "number", description: "Chapter/SI number" },
      },
      required: ["type", "year", "number"],
    },
  },
];

// ── EUR-Lex (free, no key) ───────────────────────────────────────────
const EURLEX_TOOLS = [
  {
    name: "eurlex_search",
    description: "Search EU legislation, case law, and legal documents on EUR-Lex. Covers regulations, directives, CJEU judgments.",
    inputSchema: {
      type: "object",
      properties: {
        text: { type: "string", description: "Search text" },
        type: { type: "string", description: "Document type: REG (regulation), DIR (directive), DEC (decision), JUDG (judgment)" },
        date_from: { type: "string", description: "Date from (YYYY-MM-DD)" },
        date_to: { type: "string", description: "Date to (YYYY-MM-DD)" },
        page: { type: "number" },
        pageSize: { type: "number" },
      },
      required: ["text"],
    },
  },
  {
    name: "eurlex_get_document",
    description: "Get an EU legal document by its CELEX number (e.g. '32016R0679' for GDPR, '62014CJ0362' for Schrems).",
    inputSchema: {
      type: "object",
      properties: {
        celex: { type: "string", description: "CELEX document number" },
        language: { type: "string", description: "Language code (default 'EN')" },
      },
      required: ["celex"],
    },
  },
];

// ── Open States (free API key) ───────────────────────────────────────
const OPENSTATES_TOOLS = [
  {
    name: "openstates_search_bills",
    description: "Search US state legislature bills across all 50 states. Returns bill ID, title, sponsors, actions, and status.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "Search query" },
        jurisdiction: { type: "string", description: "State (e.g. 'California', 'ca')" },
        session: { type: "string", description: "Legislative session" },
        classification: { type: "string", description: "Bill type: bill, resolution, joint resolution" },
        subject: { type: "string", description: "Subject area" },
        page: { type: "number" },
        per_page: { type: "number" },
      },
      required: ["q"],
    },
  },
  {
    name: "openstates_get_bill",
    description: "Get full bill details including all versions, votes, sponsors, and actions.",
    inputSchema: {
      type: "object",
      properties: {
        jurisdiction: { type: "string", description: "State (e.g. 'ca')" },
        session: { type: "string", description: "Session identifier" },
        identifier: { type: "string", description: "Bill identifier (e.g. 'SB 1234')" },
      },
      required: ["jurisdiction", "session", "identifier"],
    },
  },
  {
    name: "openstates_search_legislators",
    description: "Search state legislators by name, state, or chamber.",
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: "Legislator name" },
        jurisdiction: { type: "string", description: "State" },
        chamber: { type: "string", description: "Chamber: upper, lower" },
        page: { type: "number" },
      },
      required: ["name"],
    },
  },
];

// ── CanLII (free API key) ────────────────────────────────────────────
const CANLII_TOOLS = [
  {
    name: "canlii_search",
    description: "Search Canadian case law and legislation on CanLII. Covers federal and provincial courts and legislatures.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        databases: { type: "string", description: "Database filter (e.g. 'csc-scc' for Supreme Court, 'onca' for Ontario CA)" },
        resultCount: { type: "number", description: "Number of results (default 20)" },
        offset: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "canlii_get_case",
    description: "Get full case details and text from CanLII by database ID and case ID.",
    inputSchema: {
      type: "object",
      properties: {
        databaseId: { type: "string", description: "Database ID (e.g. 'csc-scc', 'onca')" },
        caseId: { type: "string", description: "Case ID from CanLII" },
      },
      required: ["databaseId", "caseId"],
    },
  },
  {
    name: "canlii_case_citations",
    description: "Get cases that cite or are cited by a given case.",
    inputSchema: {
      type: "object",
      properties: {
        databaseId: { type: "string", description: "Database ID" },
        caseId: { type: "string", description: "Case ID" },
        type: { type: "string", description: "'citedCases' or 'citingCases'" },
      },
      required: ["databaseId", "caseId"],
    },
  },
  {
    name: "canlii_get_legislation",
    description: "Get legislation text from CanLII.",
    inputSchema: {
      type: "object",
      properties: {
        databaseId: { type: "string", description: "Database ID (e.g. 'cas' for federal statutes)" },
        legislationId: { type: "string", description: "Legislation ID" },
      },
      required: ["databaseId", "legislationId"],
    },
  },
];

// ── USPTO (free, no key) ─────────────────────────────────────────────
const USPTO_TOOLS = [
  {
    name: "uspto_search_patents",
    description: "Search US patent applications and grants. Returns patent numbers, titles, abstracts, and inventors.",
    inputSchema: {
      type: "object",
      properties: {
        searchText: { type: "string", description: "Search query for patent text" },
        start: { type: "number", description: "Starting result (default 0)" },
        rows: { type: "number", description: "Number of results (default 20)" },
      },
      required: ["searchText"],
    },
  },
  {
    name: "uspto_get_patent",
    description: "Get patent details by application or patent number.",
    inputSchema: {
      type: "object",
      properties: {
        patentNumber: { type: "string", description: "Patent or application number" },
      },
      required: ["patentNumber"],
    },
  },
  {
    name: "uspto_search_trademarks",
    description: "Search US trademark registrations and applications.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Trademark search query" },
        status: { type: "string", description: "Status filter: LIVE, DEAD, all" },
        start: { type: "number" },
        rows: { type: "number" },
      },
      required: ["query"],
    },
  },
];

// ── LexisNexis (BYOK) ───────────────────────────────────────────────
const LEXIS_TOOLS = [
  {
    name: "lexis_search",
    description: "Search LexisNexis for case law, secondary sources, and legal content. Returns matching documents with citations and summaries.",
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
      properties: { document_id: { type: "string", description: "LexisNexis document ID" } },
      required: ["document_id"],
    },
  },
  {
    name: "lexis_shepards",
    description: "Check Shepard's citation treatment for a legal citation. Shows whether the case has been affirmed, distinguished, overruled, etc.",
    inputSchema: {
      type: "object",
      properties: { citation: { type: "string", description: "Legal citation (e.g. '410 U.S. 113')" } },
      required: ["citation"],
    },
  },
  {
    name: "statenet_search_bills",
    description: "Search State Net for bills and legislation by keyword, state, and session.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        state: { type: "string", description: "State abbreviation (e.g. 'CA', 'NY')" },
        session: { type: "string", description: "Legislative session" },
        status: { type: "string", description: "Bill status filter" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "statenet_get_bill",
    description: "Retrieve full bill details from State Net including text, history, and sponsors.",
    inputSchema: {
      type: "object",
      properties: { bill_id: { type: "string", description: "Bill identifier" } },
      required: ["bill_id"],
    },
  },
  {
    name: "statenet_search_regulations",
    description: "Search State Net for federal register and state regulations.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        agency: { type: "string", description: "Issuing agency" },
        date_from: { type: "string" },
        date_to: { type: "string" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "statenet_get_statute",
    description: "Retrieve statute text from State Net by citation.",
    inputSchema: {
      type: "object",
      properties: { citation: { type: "string", description: "Statute citation" } },
      required: ["citation"],
    },
  },
  {
    name: "lexmachina_search_cases",
    description: "Search Lex Machina litigation analytics — find cases by party, attorney, judge, court, or case type. Returns resolutions, damages, and timing.",
    inputSchema: {
      type: "object",
      properties: {
        party: { type: "string" }, attorney: { type: "string" },
        judge: { type: "string" }, court: { type: "string" },
        case_type: { type: "string", description: "Case type (patent, antitrust, etc.)" },
        date_from: { type: "string" }, date_to: { type: "string" },
        limit: { type: "number" },
      },
    },
  },
  {
    name: "lexmachina_case_details",
    description: "Get full Lex Machina case analytics: resolutions, damages awarded, remedies, and timing.",
    inputSchema: {
      type: "object",
      properties: { case_id: { type: "string", description: "Lex Machina case ID" } },
      required: ["case_id"],
    },
  },
  {
    name: "lexmachina_judge_profile",
    description: "Get Lex Machina judge analytics: ruling patterns, case duration, and outcomes by case type.",
    inputSchema: {
      type: "object",
      properties: { judge_id: { type: "string", description: "Judge identifier" } },
      required: ["judge_id"],
    },
  },
  {
    name: "lexmachina_party_history",
    description: "Get a party's Lex Machina litigation history: cases filed, win rates, typical damages.",
    inputSchema: {
      type: "object",
      properties: { party_name: { type: "string", description: "Party name" } },
      required: ["party_name"],
    },
  },
  {
    name: "intelligize_search_filings",
    description: "Search Intelligize SEC filings (10-K, 10-Q, 8-K, proxy) by company and type.",
    inputSchema: {
      type: "object",
      properties: {
        company: { type: "string" }, filing_type: { type: "string" },
        date_from: { type: "string" }, date_to: { type: "string" },
        limit: { type: "number" },
      },
    },
  },
  {
    name: "intelligize_get_filing",
    description: "Retrieve an Intelligize SEC filing by ID, optionally a specific section.",
    inputSchema: {
      type: "object",
      properties: {
        filing_id: { type: "string", description: "Filing identifier" },
        section: { type: "string", description: "Specific section" },
      },
      required: ["filing_id"],
    },
  },
  {
    name: "intelligize_search_clauses",
    description: "Find specific clause language across SEC filings on Intelligize.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string" }, filing_type: { type: "string" }, limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "cognitive_resolve_judge",
    description: "Resolve a judge name to a canonical entity with court assignments and metadata.",
    inputSchema: {
      type: "object",
      properties: { name: { type: "string", description: "Judge name" } },
      required: ["name"],
    },
  },
  {
    name: "cognitive_resolve_court",
    description: "Resolve a court name or abbreviation to a canonical entity.",
    inputSchema: {
      type: "object",
      properties: { name: { type: "string", description: "Court name or abbreviation" } },
      required: ["name"],
    },
  },
  {
    name: "cognitive_legal_define",
    description: "Look up a legal term definition with context and related terms.",
    inputSchema: {
      type: "object",
      properties: { term: { type: "string", description: "Legal term" } },
      required: ["term"],
    },
  },
  {
    name: "cognitive_redact_pii",
    description: "Detect and redact personally identifiable information from text.",
    inputSchema: {
      type: "object",
      properties: { text: { type: "string", description: "Text to redact" } },
      required: ["text"],
    },
  },
  {
    name: "cognitive_translate",
    description: "Translate legal text to a target language.",
    inputSchema: {
      type: "object",
      properties: {
        text: { type: "string" },
        target_language: { type: "string", description: "Target language code (e.g. 'es', 'fr')" },
      },
      required: ["text", "target_language"],
    },
  },
];

// ── Westlaw / Thomson Reuters (BYOK) ────────────────────────────────
const WESTLAW_TOOLS = [
  {
    name: "westlaw_search",
    description: "Search Westlaw for case law, statutes, regulations, and secondary sources. The other half of the legal research duopoly alongside LexisNexis.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query (natural language or Terms & Connectors)" },
        jurisdiction: { type: "string", description: "Jurisdiction filter" },
        content_type: { type: "string", description: "Content: cases, statutes, regulations, secondary, all" },
        date_from: { type: "string" },
        date_to: { type: "string" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "westlaw_get_document",
    description: "Retrieve a document from Westlaw by its WestlawNext document ID.",
    inputSchema: {
      type: "object",
      properties: { document_id: { type: "string", description: "Westlaw document ID" } },
      required: ["document_id"],
    },
  },
  {
    name: "westlaw_keycite",
    description: "KeyCite citation verification — Westlaw's equivalent of Shepard's. Check if a case is still good law, see citing references and negative treatment.",
    inputSchema: {
      type: "object",
      properties: { citation: { type: "string", description: "Legal citation" } },
      required: ["citation"],
    },
  },
  {
    name: "westlaw_practical_law",
    description: "Search Practical Law for practice notes, standard documents, checklists, and how-to guides. The industry-standard legal know-how platform.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        resource_type: { type: "string", description: "Type: practice-note, standard-document, checklist, toolkit" },
        practice_area: { type: "string", description: "Practice area filter" },
        jurisdiction: { type: "string", description: "Jurisdiction" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "westlaw_dockets",
    description: "Search court dockets via Westlaw. Broader coverage than RECAP for state courts.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        court: { type: "string", description: "Court filter" },
        party: { type: "string", description: "Party name" },
        filed_after: { type: "string" },
        filed_before: { type: "string" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "westlaw_litigation_analytics",
    description: "Westlaw Edge Litigation Analytics — judge profiles, case outcomes, motion success rates, damages data, and time-to-resolution statistics.",
    inputSchema: {
      type: "object",
      properties: {
        query_type: { type: "string", description: "Type: judge, court, party, attorney, case-type" },
        query: { type: "string", description: "Name or search term" },
        case_type: { type: "string", description: "Case type filter" },
        date_from: { type: "string" },
        date_to: { type: "string" },
      },
      required: ["query_type", "query"],
    },
  },
];

// ── Clio (BYOK) ─────────────────────────────────────────────────────
const CLIO_TOOLS = [
  {
    name: "clio_search_matters",
    description: "Search matters (cases) in Clio practice management. Returns matter details, status, responsible attorney, and client.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        status: { type: "string", description: "Status: open, pending, closed" },
        client_id: { type: "number", description: "Filter by client ID" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "clio_get_matter",
    description: "Get full details of a Clio matter including custom fields, relationships, and billing summary.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "number", description: "Matter ID" } },
      required: ["id"],
    },
  },
  {
    name: "clio_create_time_entry",
    description: "Log a time entry in Clio for billing. Supports LEDES/UTBMS activity codes.",
    inputSchema: {
      type: "object",
      properties: {
        matter_id: { type: "number", description: "Matter ID to bill to" },
        date: { type: "string", description: "Date (YYYY-MM-DD)" },
        quantity: { type: "number", description: "Hours (e.g. 0.5 for 30 min)" },
        description: { type: "string", description: "Work description" },
        activity_code: { type: "string", description: "UTBMS activity code (e.g. 'A101')" },
      },
      required: ["matter_id", "date", "quantity", "description"],
    },
  },
  {
    name: "clio_search_contacts",
    description: "Search contacts (clients, opposing counsel, witnesses) in Clio.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Name or search query" },
        type: { type: "string", description: "Type: Person, Company" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "clio_search_documents",
    description: "Search documents stored in Clio.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        matter_id: { type: "number", description: "Filter by matter" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "clio_create_task",
    description: "Create a task in Clio assigned to a matter with due date and assignee.",
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: "Task name" },
        matter_id: { type: "number", description: "Associated matter" },
        due_date: { type: "string", description: "Due date (YYYY-MM-DD)" },
        assignee_id: { type: "number", description: "Assignee user ID" },
        description: { type: "string", description: "Task description" },
        priority: { type: "string", description: "Priority: High, Normal, Low" },
      },
      required: ["name"],
    },
  },
];

// ── iManage (BYOK) ──────────────────────────────────────────────────
const IMANAGE_TOOLS = [
  {
    name: "imanage_search",
    description: "Search documents in iManage Work DMS. Used by most Am Law 200 firms.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
        scope: { type: "string", description: "Search scope: content, metadata, all" },
        doc_class: { type: "string", description: "Document class filter" },
        author: { type: "string", description: "Author filter" },
        date_from: { type: "string" },
        date_to: { type: "string" },
        limit: { type: "number" },
      },
      required: ["query"],
    },
  },
  {
    name: "imanage_get_document",
    description: "Retrieve a document from iManage by its document ID. Returns metadata and content.",
    inputSchema: {
      type: "object",
      properties: { document_id: { type: "string", description: "iManage document ID" } },
      required: ["document_id"],
    },
  },
  {
    name: "imanage_get_document_content",
    description: "Download the actual file content of an iManage document.",
    inputSchema: {
      type: "object",
      properties: { document_id: { type: "string", description: "iManage document ID" } },
      required: ["document_id"],
    },
  },
  {
    name: "imanage_upload",
    description: "Upload a document to iManage Work.",
    inputSchema: {
      type: "object",
      properties: {
        workspace_id: { type: "string", description: "Target workspace/folder ID" },
        name: { type: "string", description: "Document name" },
        content: { type: "string", description: "Document content (text)" },
        doc_class: { type: "string", description: "Document class" },
      },
      required: ["workspace_id", "name", "content"],
    },
  },
  {
    name: "imanage_checkout",
    description: "Check out a document for editing (locks it).",
    inputSchema: {
      type: "object",
      properties: { document_id: { type: "string", description: "Document ID" } },
      required: ["document_id"],
    },
  },
  {
    name: "imanage_checkin",
    description: "Check in a document after editing (unlocks it).",
    inputSchema: {
      type: "object",
      properties: {
        document_id: { type: "string", description: "Document ID" },
        content: { type: "string", description: "Updated content" },
        comment: { type: "string", description: "Version comment" },
      },
      required: ["document_id"],
    },
  },
];

// ── NetDocuments (BYOK) ─────────────────────────────────────────────
const NETDOCUMENTS_TOOLS = [
  {
    name: "netdocuments_search",
    description: "Search documents in NetDocuments cloud DMS.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "Search query" },
        cabinet: { type: "string", description: "Cabinet/repository filter" },
        extension: { type: "string", description: "File extension filter (docx, pdf)" },
        modified_after: { type: "string", description: "Modified after (YYYY-MM-DD)" },
        limit: { type: "number" },
      },
      required: ["q"],
    },
  },
  {
    name: "netdocuments_get_document",
    description: "Get document metadata and properties from NetDocuments.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "string", description: "NetDocuments envelope ID" } },
      required: ["id"],
    },
  },
  {
    name: "netdocuments_get_content",
    description: "Download document content from NetDocuments.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "string", description: "NetDocuments envelope ID" } },
      required: ["id"],
    },
  },
  {
    name: "netdocuments_upload",
    description: "Upload a document to NetDocuments.",
    inputSchema: {
      type: "object",
      properties: {
        cabinet: { type: "string", description: "Target cabinet ID" },
        name: { type: "string", description: "Document name" },
        content: { type: "string", description: "Document content (text)" },
        profile: { type: "object", description: "Profile attributes (metadata)" },
      },
      required: ["cabinet", "name", "content"],
    },
  },
];

// ═══════════════════════════════════════════════════════════════════════
// TOOL HANDLERS
// ═══════════════════════════════════════════════════════════════════════

async function handleTool(name, args) {
  const key = cacheKey(name, args);
  const hit = cached(key);
  if (hit) return hit;

  let result;

  switch (name) {
    // ── CourtListener ──────────────────────────────────────────────
    case "courtlistener_search_opinions": {
      const params = { q: args.q, type: "o" };
      if (args.court) params.court = args.court;
      if (args.filed_after) params.filed_after = args.filed_after;
      if (args.filed_before) params.filed_before = args.filed_before;
      if (args.cited_gt) params.cited_gt = args.cited_gt;
      if (args.ordering) params.ordering = args.ordering;
      if (args.page) params.page = args.page;
      result = await fetchJSON(`${COURTLISTENER}/search/?${qs(params)}`);
      break;
    }
    case "courtlistener_get_opinion":
      result = await fetchJSON(`${COURTLISTENER}/clusters/${validateId(args.id)}/`);
      break;
    case "courtlistener_search_dockets": {
      const params = { q: args.q, type: "d" };
      if (args.court) params.court = args.court;
      if (args.filed_after) params.filed_after = args.filed_after;
      if (args.filed_before) params.filed_before = args.filed_before;
      if (args.nature_of_suit) params.nature_of_suit = args.nature_of_suit;
      if (args.page) params.page = args.page;
      result = await fetchJSON(`${COURTLISTENER}/search/?${qs(params)}`);
      break;
    }
    case "courtlistener_get_docket":
      result = await fetchJSON(`${COURTLISTENER}/dockets/${validateId(args.id)}/`);
      break;
    case "courtlistener_search_judges": {
      const params = { q: args.q, type: "p" };
      if (args.court) params.court = args.court;
      if (args.page) params.page = args.page;
      result = await fetchJSON(`${COURTLISTENER}/search/?${qs(params)}`);
      break;
    }
    case "courtlistener_get_judge":
      result = await fetchJSON(`${COURTLISTENER}/people/${validateId(args.id)}/`);
      break;
    case "courtlistener_search_oral_arguments": {
      const params = { q: args.q, type: "oa" };
      if (args.court) params.court = args.court;
      if (args.argued_after) params.argued_after = args.argued_after;
      if (args.argued_before) params.argued_before = args.argued_before;
      if (args.page) params.page = args.page;
      result = await fetchJSON(`${COURTLISTENER}/search/?${qs(params)}`);
      break;
    }
    case "courtlistener_search_recap_documents": {
      const params = { q: args.q, type: "rd" };
      if (args.docket_id) params.docket_id = args.docket_id;
      if (args.description) params.description = args.description;
      if (args.page) params.page = args.page;
      result = await fetchJSON(`${COURTLISTENER}/search/?${qs(params)}`);
      break;
    }
    case "courtlistener_citation_lookup": {
      result = await fetchJSON(`${COURTLISTENER}/search/?q=${encodeURIComponent(args.cite)}&type=o`);
      break;
    }

    // ── SEC EDGAR ──────────────────────────────────────────────────
    case "edgar_fulltext_search": {
      const params = { q: args.q };
      if (args.dateRange) params.dateRange = args.dateRange;
      if (args.startdt) params.startdt = args.startdt;
      if (args.enddt) params.enddt = args.enddt;
      if (args.forms) params.forms = args.forms;
      if (args.from) params.from = args.from;
      result = await fetchJSON(`${EDGAR}/search-index?${qs(params)}`);
      break;
    }
    case "edgar_company_filings": {
      const cik = validateId(args.cik).padStart(10, "0");
      const params = {};
      if (args.type) params.type = args.type;
      if (args.count) params.count = args.count;
      result = await fetchJSON(`${EDGAR_DATA}/submissions/CIK${cik}.json`);
      break;
    }
    case "edgar_company_facts": {
      const cik = validateId(args.cik).padStart(10, "0");
      result = await fetchJSON(`${EDGAR_DATA}/api/xbrl/companyfacts/CIK${cik}.json`);
      break;
    }
    case "edgar_company_concept": {
      const cik = validateId(args.cik).padStart(10, "0");
      result = await fetchJSON(`${EDGAR_DATA}/api/xbrl/companyconcept/CIK${cik}/${validateId(args.taxonomy)}/${validateId(args.concept)}.json`);
      break;
    }
    case "edgar_resolve_ticker": {
      const data = await fetchJSON(`${EDGAR}/search-index?q=${encodeURIComponent(args.query)}&dateRange=custom&startdt=2020-01-01`);
      const hits = (data.hits?.hits || []).slice(0, 5).map(h => ({
        name: h._source?.entity_name,
        cik: h._source?.entity_id,
        ticker: h._source?.ticker,
      }));
      result = { matches: hits };
      break;
    }

    // ── Federal Register ───────────────────────────────────────────
    case "federal_register_search": {
      const params = new URLSearchParams();
      const c = args.conditions || {};
      if (c.term) params.set("conditions[term]", c.term);
      if (c.agencies) c.agencies.forEach(a => params.append("conditions[agencies][]", a));
      if (c.type) c.type.forEach(t => params.append("conditions[type][]", t));
      if (c.publication_date?.gte) params.set("conditions[publication_date][gte]", c.publication_date.gte);
      if (c.publication_date?.lte) params.set("conditions[publication_date][lte]", c.publication_date.lte);
      if (args.page) params.set("page", args.page);
      if (args.per_page) params.set("per_page", args.per_page);
      if (args.order) params.set("order", args.order);
      result = await fetchJSON(`${FED_REGISTER}/documents?${params}`);
      break;
    }
    case "federal_register_get_document":
      result = await fetchJSON(`${FED_REGISTER}/documents/${validateId(args.document_number)}`);
      break;
    case "federal_register_get_agency":
      result = await fetchJSON(`${FED_REGISTER}/agencies/${validateId(args.slug)}`);
      break;

    // ── regulations.gov ────────────────────────────────────────────
    case "regulations_search_documents": {
      const apiKey = REGULATIONS_KEY;
      if (!apiKey) throw new Error("regulations.gov API key not configured. Get a free key at https://api.data.gov/signup/");
      const params = new URLSearchParams();
      const f = args.filter || {};
      if (f.searchTerm) params.set("filter[searchTerm]", f.searchTerm);
      if (f.agencyId) params.set("filter[agencyId]", f.agencyId);
      if (f.documentType) params.set("filter[documentType]", f.documentType);
      if (f.postedDate) params.set("filter[postedDate]", f.postedDate);
      if (args.page) params.set("page[number]", args.page);
      if (args.pageSize) params.set("page[size]", args.pageSize);
      result = await fetchJSON(`${REGULATIONS}/documents?${params}`, {
        headers: { "X-Api-Key": apiKey },
      });
      break;
    }
    case "regulations_get_document": {
      const apiKey = REGULATIONS_KEY;
      if (!apiKey) throw new Error("regulations.gov API key not configured. Get a free key at https://api.data.gov/signup/");
      result = await fetchJSON(`${REGULATIONS}/documents/${validateId(args.documentId)}`, {
        headers: { "X-Api-Key": apiKey },
      });
      break;
    }
    case "regulations_search_dockets": {
      const apiKey = REGULATIONS_KEY;
      if (!apiKey) throw new Error("regulations.gov API key not configured. Get a free key at https://api.data.gov/signup/");
      const params = new URLSearchParams();
      const f = args.filter || {};
      if (f.searchTerm) params.set("filter[searchTerm]", f.searchTerm);
      if (f.agencyId) params.set("filter[agencyId]", f.agencyId);
      if (f.docketType) params.set("filter[docketType]", f.docketType);
      if (args.page) params.set("page[number]", args.page);
      result = await fetchJSON(`${REGULATIONS}/dockets?${params}`, {
        headers: { "X-Api-Key": apiKey },
      });
      break;
    }
    case "regulations_get_comments": {
      const apiKey = REGULATIONS_KEY;
      if (!apiKey) throw new Error("regulations.gov API key not configured. Get a free key at https://api.data.gov/signup/");
      const params = new URLSearchParams();
      const f = args.filter || {};
      if (f.commentOnId) params.set("filter[commentOnId]", f.commentOnId);
      if (f.searchTerm) params.set("filter[searchTerm]", f.searchTerm);
      if (args.page) params.set("page[number]", args.page);
      if (args.pageSize) params.set("page[size]", args.pageSize);
      result = await fetchJSON(`${REGULATIONS}/comments?${params}`, {
        headers: { "X-Api-Key": apiKey },
      });
      break;
    }

    // ── Congress.gov ───────────────────────────────────────────────
    case "congress_search_bills": {
      const apiKey = CONGRESS_KEY;
      if (!apiKey) throw new Error("Congress.gov API key not configured. Get a free key at https://api.congress.gov/sign-up/");
      const params = { query: args.query, format: "json" };
      if (args.offset) params.offset = args.offset;
      if (args.limit) params.limit = args.limit;
      result = await fetchJSON(`${CONGRESS}/bill?${qs(params)}`, {
        headers: { "X-Api-Key": apiKey },
      });
      break;
    }
    case "congress_get_bill": {
      const apiKey = CONGRESS_KEY;
      if (!apiKey) throw new Error("Congress.gov API key not configured.");
      result = await fetchJSON(`${CONGRESS}/bill/${validateId(args.congress)}/${validateId(args.type)}/${validateId(args.number)}?format=json`, {
        headers: { "X-Api-Key": apiKey },
      });
      break;
    }
    case "congress_get_bill_text": {
      const apiKey = CONGRESS_KEY;
      if (!apiKey) throw new Error("Congress.gov API key not configured.");
      result = await fetchJSON(`${CONGRESS}/bill/${validateId(args.congress)}/${validateId(args.type)}/${validateId(args.number)}/text?format=json`, {
        headers: { "X-Api-Key": apiKey },
      });
      break;
    }
    case "congress_search_members": {
      const apiKey = CONGRESS_KEY;
      if (!apiKey) throw new Error("Congress.gov API key not configured.");
      const params = { query: args.query, format: "json" };
      if (args.currentMember !== undefined) params.currentMember = args.currentMember;
      if (args.offset) params.offset = args.offset;
      if (args.limit) params.limit = args.limit;
      result = await fetchJSON(`${CONGRESS}/member?${qs(params)}`, {
        headers: { "X-Api-Key": apiKey },
      });
      break;
    }
    case "congress_get_member": {
      const apiKey = CONGRESS_KEY;
      if (!apiKey) throw new Error("Congress.gov API key not configured.");
      result = await fetchJSON(`${CONGRESS}/member/${validateId(args.bioguideId)}?format=json`, {
        headers: { "X-Api-Key": apiKey },
      });
      break;
    }

    // ── UK Legislation ─────────────────────────────────────────────
    case "uk_legislation_search": {
      const params = { text: args.query };
      if (args.type) params.type = args.type;
      if (args.year) params.year = args.year;
      if (args.page) params.page = args.page;
      result = await fetchJSON(`${UK_LEG}/search?${qs(params)}`, {
        headers: { Accept: "application/json" },
      });
      break;
    }
    case "uk_legislation_get": {
      const section = args.section ? "/" + args.section.split("/").map(s => validateId(s)).join("/") : "";
      const url = `${UK_LEG}/${validateId(args.type)}/${validateId(args.year)}/${validateId(args.number)}${section}/data.json`;
      result = await fetchJSON(url);
      break;
    }
    case "uk_legislation_changes": {
      result = await fetchJSON(`${UK_LEG}/${validateId(args.type)}/${validateId(args.year)}/${validateId(args.number)}/changes/data.json`);
      break;
    }

    // ── EUR-Lex ────────────────────────────────────────────────────
    case "eurlex_search": {
      // EUR-Lex search via the public search API
      const params = { text: args.text, page: args.page || 1, pageSize: args.pageSize || 20 };
      if (args.type) params.type = args.type;
      if (args.date_from) params.date_from = args.date_from;
      if (args.date_to) params.date_to = args.date_to;
      // Use the web search endpoint as the REST API has limited public access
      const searchUrl = `https://eur-lex.europa.eu/search.html?textScope=ti-te&text=${encodeURIComponent(args.text)}&qid=1&type=quick&lang=en&DTS_SUBDOM=LEGISLATION`;
      // Fall back to scraping the search page summary
      const resp = await fetch(searchUrl, { headers: { "User-Agent": UA, Accept: "text/html" } });
      const html = await resp.text();
      // Extract CELEX numbers from results
      const celexMatches = [...html.matchAll(/CELEX[^"]*?(\d{5}[A-Z]\d{4})/g)].map(m => m[1]);
      result = { query: args.text, celex_numbers: [...new Set(celexMatches)].slice(0, 20), note: "Use eurlex_get_document with a CELEX number to get full text" };
      break;
    }
    case "eurlex_get_document": {
      const lang = validateId(args.language || "EN");
      const url = `https://eur-lex.europa.eu/legal-content/${lang}/TXT/HTML/?uri=CELEX:${validateId(args.celex)}`;
      const resp = await fetch(url, { headers: { "User-Agent": UA } });
      if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`);
      const html = await resp.text();
      // Strip HTML tags for a cleaner text version
      const text = html.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim();
      result = { celex: args.celex, language: lang, text: text.slice(0, 50000) };
      break;
    }

    // ── Open States ────────────────────────────────────────────────
    case "openstates_search_bills": {
      if (!OPENSTATES_KEY) throw new Error("Open States API key not configured. Get a free key at https://openstates.org/accounts/signup/");
      const params = { q: args.q };
      if (args.jurisdiction) params.jurisdiction = args.jurisdiction;
      if (args.session) params.session = args.session;
      if (args.classification) params.classification = args.classification;
      if (args.subject) params.subject = args.subject;
      if (args.page) params.page = args.page;
      if (args.per_page) params.per_page = args.per_page;
      result = await fetchJSON(`${OPENSTATES}/bills?${qs(params)}`, {
        headers: { "X-API-KEY": OPENSTATES_KEY },
      });
      break;
    }
    case "openstates_get_bill": {
      if (!OPENSTATES_KEY) throw new Error("Open States API key not configured.");
      result = await fetchJSON(
        `${OPENSTATES}/bills/${validateId(args.jurisdiction)}/${validateId(args.session)}/${encodeURIComponent(args.identifier)}`,
        { headers: { "X-API-KEY": OPENSTATES_KEY } },
      );
      break;
    }
    case "openstates_search_legislators": {
      if (!OPENSTATES_KEY) throw new Error("Open States API key not configured.");
      const params = { name: args.name };
      if (args.jurisdiction) params.jurisdiction = args.jurisdiction;
      if (args.chamber) params.org_classification = args.chamber;
      if (args.page) params.page = args.page;
      result = await fetchJSON(`${OPENSTATES}/people?${qs(params)}`, {
        headers: { "X-API-KEY": OPENSTATES_KEY },
      });
      break;
    }

    // ── CanLII (query param auth only — no header auth supported) ──
    case "canlii_search": {
      if (!CANLII_KEY) throw new Error("CanLII API key not configured. Request access at https://www.canlii.org/en/tools/api.html");
      const params = { api_key: CANLII_KEY, query: args.query };
      if (args.databases) params.databases = args.databases;
      if (args.resultCount) params.resultCount = args.resultCount;
      if (args.offset) params.offset = args.offset;
      result = await fetchJSON(`${CANLII_BASE}/search?${qs(params)}`);
      break;
    }
    case "canlii_get_case": {
      if (!CANLII_KEY) throw new Error("CanLII API key not configured.");
      result = await fetchJSON(`${CANLII_BASE}/caseBrowse/${validateId(args.databaseId)}/${validateId(args.caseId)}?api_key=${CANLII_KEY}`);
      break;
    }
    case "canlii_case_citations": {
      if (!CANLII_KEY) throw new Error("CanLII API key not configured.");
      const type = args.type || "citedCases";
      result = await fetchJSON(`${CANLII_BASE}/caseCitator/${validateId(args.databaseId)}/${validateId(args.caseId)}/${validateId(type)}?api_key=${CANLII_KEY}`);
      break;
    }
    case "canlii_get_legislation": {
      if (!CANLII_KEY) throw new Error("CanLII API key not configured.");
      result = await fetchJSON(`${CANLII_BASE}/legislationBrowse/${validateId(args.databaseId)}/${validateId(args.legislationId)}?api_key=${CANLII_KEY}`);
      break;
    }

    // ── USPTO ──────────────────────────────────────────────────────
    case "uspto_search_patents": {
      const params = { searchText: args.searchText, start: args.start || 0, rows: args.rows || 20 };
      result = await fetchJSON(`${USPTO}/application?${qs(params)}`);
      break;
    }
    case "uspto_get_patent":
      result = await fetchJSON(`${USPTO}/application/${validateId(args.patentNumber)}`);
      break;
    case "uspto_search_trademarks": {
      const params = new URLSearchParams();
      params.set("searchText", args.query);
      if (args.status) params.set("status", args.status);
      if (args.start) params.set("start", args.start);
      if (args.rows) params.set("rows", args.rows);
      result = await fetchJSON(`https://developer.uspto.gov/trademark-api/v1/trademarks?${params}`);
      break;
    }

    // ── LexisNexis (BYOK) ─────────────────────────────────────────
    case "lexis_search":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(LEXIS_BASE, "/search", LEXIS_KEY, "POST", args);
      break;
    case "lexis_retrieve":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(LEXIS_BASE, `/documents/${validateId(args.document_id)}`, LEXIS_KEY);
      break;
    case "lexis_shepards":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(LEXIS_BASE, `/shepards/${encodeURIComponent(args.citation)}`, LEXIS_KEY);
      break;
    case "statenet_search_bills":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(STATENET_BASE, "/bills/search", LEXIS_KEY, "POST", args);
      break;
    case "statenet_get_bill":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(STATENET_BASE, `/bills/${validateId(args.bill_id)}`, LEXIS_KEY);
      break;
    case "statenet_search_regulations":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(STATENET_BASE, "/regulations/search", LEXIS_KEY, "POST", args);
      break;
    case "statenet_get_statute":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(STATENET_BASE, `/statutes/${encodeURIComponent(args.citation)}`, LEXIS_KEY);
      break;
    case "lexmachina_search_cases":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(LEXMACHINA_BASE, "/cases/search", LEXIS_KEY, "POST", args);
      break;
    case "lexmachina_case_details":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(LEXMACHINA_BASE, `/cases/${validateId(args.case_id)}`, LEXIS_KEY);
      break;
    case "lexmachina_judge_profile":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(LEXMACHINA_BASE, `/judges/${validateId(args.judge_id)}`, LEXIS_KEY);
      break;
    case "lexmachina_party_history": {
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(LEXMACHINA_BASE, `/parties?${qs({ name: args.party_name })}`, LEXIS_KEY);
      break;
    }
    case "intelligize_search_filings":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(INTELLIGIZE_BASE, "/filings/search", LEXIS_KEY, "POST", args);
      break;
    case "intelligize_get_filing": {
      requireKey("LexisNexis", LEXIS_KEY);
      const fid = validateId(args.filing_id);
      const path = args.section
        ? `/filings/${fid}?section=${encodeURIComponent(args.section)}`
        : `/filings/${fid}`;
      result = await authedCall(INTELLIGIZE_BASE, path, LEXIS_KEY);
      break;
    }
    case "intelligize_search_clauses":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(INTELLIGIZE_BASE, "/clauses/search", LEXIS_KEY, "POST", args);
      break;
    case "cognitive_resolve_judge":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(COGNITIVE_BASE, `/entities/judges?${qs({ name: args.name })}`, LEXIS_KEY);
      break;
    case "cognitive_resolve_court":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(COGNITIVE_BASE, `/entities/courts?${qs({ name: args.name })}`, LEXIS_KEY);
      break;
    case "cognitive_legal_define":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(COGNITIVE_BASE, `/dictionary/${encodeURIComponent(args.term)}`, LEXIS_KEY);
      break;
    case "cognitive_redact_pii":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(COGNITIVE_BASE, "/redact", LEXIS_KEY, "POST", { text: args.text });
      break;
    case "cognitive_translate":
      requireKey("LexisNexis", LEXIS_KEY);
      result = await authedCall(COGNITIVE_BASE, "/translate", LEXIS_KEY, "POST", { text: args.text, target_language: args.target_language });
      break;

    // ── Westlaw (BYOK) ────────────────────────────────────────────
    case "westlaw_search":
      requireKey("Westlaw", WESTLAW_KEY);
      result = await authedCall(WESTLAW_BASE, "/search", WESTLAW_KEY, "POST", args);
      break;
    case "westlaw_get_document":
      requireKey("Westlaw", WESTLAW_KEY);
      result = await authedCall(WESTLAW_BASE, `/documents/${validateId(args.document_id)}`, WESTLAW_KEY);
      break;
    case "westlaw_keycite":
      requireKey("Westlaw", WESTLAW_KEY);
      result = await authedCall(WESTLAW_BASE, `/keycite/${encodeURIComponent(args.citation)}`, WESTLAW_KEY);
      break;
    case "westlaw_practical_law":
      requireKey("Westlaw", WESTLAW_KEY);
      result = await authedCall(WESTLAW_BASE, "/practical-law/search", WESTLAW_KEY, "POST", args);
      break;
    case "westlaw_dockets":
      requireKey("Westlaw", WESTLAW_KEY);
      result = await authedCall(WESTLAW_BASE, "/dockets/search", WESTLAW_KEY, "POST", args);
      break;
    case "westlaw_litigation_analytics":
      requireKey("Westlaw", WESTLAW_KEY);
      result = await authedCall(WESTLAW_BASE, `/analytics/${validateId(args.query_type)}`, WESTLAW_KEY, "POST", args);
      break;

    // ── Clio (BYOK) ───────────────────────────────────────────────
    case "clio_search_matters": {
      requireKey("Clio", CLIO_KEY);
      const params = { query: args.query };
      if (args.status) params.status = args.status;
      if (args.client_id) params.client_id = args.client_id;
      if (args.limit) params.limit = args.limit;
      result = await authedCall(CLIO_BASE, `/matters?${qs(params)}`, CLIO_KEY);
      break;
    }
    case "clio_get_matter":
      requireKey("Clio", CLIO_KEY);
      result = await authedCall(CLIO_BASE, `/matters/${validateId(args.id)}`, CLIO_KEY);
      break;
    case "clio_create_time_entry":
      requireKey("Clio", CLIO_KEY);
      result = await authedCall(CLIO_BASE, "/activities", CLIO_KEY, "POST", {
        data: {
          matter: { id: args.matter_id },
          date: args.date,
          quantity: args.quantity,
          note: args.description,
          activity_description: { id: args.activity_code },
        },
      });
      break;
    case "clio_search_contacts": {
      requireKey("Clio", CLIO_KEY);
      const params = { query: args.query };
      if (args.type) params.type = args.type;
      if (args.limit) params.limit = args.limit;
      result = await authedCall(CLIO_BASE, `/contacts?${qs(params)}`, CLIO_KEY);
      break;
    }
    case "clio_search_documents": {
      requireKey("Clio", CLIO_KEY);
      const params = { query: args.query };
      if (args.matter_id) params.matter_id = args.matter_id;
      if (args.limit) params.limit = args.limit;
      result = await authedCall(CLIO_BASE, `/documents?${qs(params)}`, CLIO_KEY);
      break;
    }
    case "clio_create_task":
      requireKey("Clio", CLIO_KEY);
      result = await authedCall(CLIO_BASE, "/tasks", CLIO_KEY, "POST", {
        data: {
          name: args.name,
          matter: args.matter_id ? { id: args.matter_id } : undefined,
          due_at: args.due_date,
          assignee: args.assignee_id ? { id: args.assignee_id } : undefined,
          description: args.description,
          priority: args.priority || "Normal",
        },
      });
      break;

    // ── iManage (BYOK) ────────────────────────────────────────────
    case "imanage_search":
      requireKey("iManage", IMANAGE_KEY);
      result = await authedCall(IMANAGE_BASE, "/documents/search", IMANAGE_KEY, "POST", args);
      break;
    case "imanage_get_document":
      requireKey("iManage", IMANAGE_KEY);
      result = await authedCall(IMANAGE_BASE, `/documents/${validateId(args.document_id)}`, IMANAGE_KEY);
      break;
    case "imanage_get_document_content":
      requireKey("iManage", IMANAGE_KEY);
      result = await authedCall(IMANAGE_BASE, `/documents/${validateId(args.document_id)}/download`, IMANAGE_KEY);
      break;
    case "imanage_upload":
      requireKey("iManage", IMANAGE_KEY);
      result = await authedCall(IMANAGE_BASE, `/workspaces/${validateId(args.workspace_id)}/documents`, IMANAGE_KEY, "POST", {
        name: args.name, content: args.content, doc_class: args.doc_class,
      });
      break;
    case "imanage_checkout":
      requireKey("iManage", IMANAGE_KEY);
      result = await authedCall(IMANAGE_BASE, `/documents/${validateId(args.document_id)}/lock`, IMANAGE_KEY, "POST");
      break;
    case "imanage_checkin":
      requireKey("iManage", IMANAGE_KEY);
      result = await authedCall(IMANAGE_BASE, `/documents/${validateId(args.document_id)}`, IMANAGE_KEY, "PUT", {
        content: args.content, comment: args.comment,
      });
      break;

    // ── NetDocuments (BYOK) ───────────────────────────────────────
    case "netdocuments_search": {
      requireKey("NetDocuments", NETDOCUMENTS_KEY);
      const params = { q: args.q };
      if (args.cabinet) params.cabinet = args.cabinet;
      if (args.extension) params.extension = args.extension;
      if (args.modified_after) params.modified_after = args.modified_after;
      if (args.limit) params.limit = args.limit;
      result = await authedCall(NETDOCS_BASE, `/search?${qs(params)}`, NETDOCUMENTS_KEY);
      break;
    }
    case "netdocuments_get_document":
      requireKey("NetDocuments", NETDOCUMENTS_KEY);
      result = await authedCall(NETDOCS_BASE, `/documents/${validateId(args.id)}`, NETDOCUMENTS_KEY);
      break;
    case "netdocuments_get_content":
      requireKey("NetDocuments", NETDOCUMENTS_KEY);
      result = await authedCall(NETDOCS_BASE, `/documents/${validateId(args.id)}/content`, NETDOCUMENTS_KEY);
      break;
    case "netdocuments_upload":
      requireKey("NetDocuments", NETDOCUMENTS_KEY);
      result = await authedCall(NETDOCS_BASE, `/cabinets/${validateId(args.cabinet)}/documents`, NETDOCUMENTS_KEY, "POST", {
        name: args.name, content: args.content, profile: args.profile,
      });
      break;

    default:
      throw new Error(`Unknown tool: ${name}`);
  }

  return setCache(key, result);
}

// ═══════════════════════════════════════════════════════════════════════
// DYNAMIC TOOL REGISTRY — only expose tools whose providers are available
// ═══════════════════════════════════════════════════════════════════════

function getAvailableTools() {
  // Free tools — always available
  const tools = [
    ...COURTLISTENER_TOOLS,
    ...EDGAR_TOOLS,
    ...FED_REGISTER_TOOLS,
    ...UK_LEG_TOOLS,
    ...USPTO_TOOLS,
  ];

  // Free-key tools — always expose, handler gives signup URL if key missing
  tools.push(...REGULATIONS_TOOLS);
  tools.push(...CONGRESS_TOOLS);
  tools.push(...OPENSTATES_TOOLS);
  tools.push(...CANLII_TOOLS);
  tools.push(...EURLEX_TOOLS);

  // BYOK — only expose when key is configured
  if (LEXIS_KEY) tools.push(...LEXIS_TOOLS);
  if (WESTLAW_KEY) tools.push(...WESTLAW_TOOLS);
  if (CLIO_KEY) tools.push(...CLIO_TOOLS);
  if (IMANAGE_KEY) tools.push(...IMANAGE_TOOLS);
  if (NETDOCUMENTS_KEY) tools.push(...NETDOCUMENTS_TOOLS);

  return tools;
}

// ═══════════════════════════════════════════════════════════════════════
// MCP SERVER
// ═══════════════════════════════════════════════════════════════════════

const server = new Server(
  { name: "lawborg-mcp", version: "0.2.0" },
  { capabilities: { tools: {} } },
);

server.setRequestHandler("tools/list", async () => ({
  tools: getAvailableTools(),
}));

server.setRequestHandler("tools/call", async (request) => {
  const { name, arguments: args } = request.params;
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
