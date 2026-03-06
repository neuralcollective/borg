#!/usr/bin/env node
// Shovels V2 MCP server — building permits and contractor data.
// Requires SHOVELS_API_KEY env var.

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";

const API_KEY = process.env.SHOVELS_API_KEY || "";
const BASE = "https://api.shovels.ai/v2";
const UA = "ShovelsMCP/0.1 (borg-build-agent)";

// ── Rate limiting ────────────────────────────────────────────────────
let timestamps = [];
const MAX_RPS = 10;
const WINDOW = 1000;

function checkRate() {
  const now = Date.now();
  timestamps = timestamps.filter(t => now - t < WINDOW);
  if (timestamps.length >= MAX_RPS) throw new Error("Rate limit: max 10 req/s");
  timestamps.push(now);
}

// ── Cache ────────────────────────────────────────────────────────────
const cache = new Map();
const CACHE_TTL = 10 * 60 * 1000;
const MAX_CACHE = 200;

function cached(key) {
  const e = cache.get(key);
  if (e && Date.now() - e.ts < CACHE_TTL) return e.val;
  if (e) cache.delete(key);
  return null;
}

function setCache(key, val) {
  cache.delete(key);
  if (cache.size >= MAX_CACHE) cache.delete(cache.keys().next().value);
  cache.set(key, { val, ts: Date.now() });
  return val;
}

// ── HTTP ─────────────────────────────────────────────────────────────
function requireKey() {
  if (!API_KEY) throw new Error("SHOVELS_API_KEY not configured. Get one at https://app.shovels.ai");
}

async function fetchAPI(path, params = {}) {
  requireKey();
  checkRate();
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== "") {
      if (Array.isArray(v)) v.forEach(i => qs.append(k, i));
      else qs.set(k, String(v));
    }
  }
  const url = `${BASE}${path}?${qs}`;
  const key = url;
  const hit = cached(key);
  if (hit) return hit;

  const resp = await fetch(url, {
    headers: { "X-API-Key": API_KEY, "User-Agent": UA, Accept: "application/json" },
  });
  if (!resp.ok) {
    const text = await resp.text().catch(() => "");
    throw new Error(`Shovels ${resp.status}: ${text.slice(0, 500)}`);
  }
  const data = await resp.json();
  return setCache(key, data);
}

// ── Tool definitions ─────────────────────────────────────────────────
const TOOLS = [
  {
    name: "shovels_search_permits",
    description: "Search building permits by jurisdiction, date range, property type, and permit type. Returns permit records with addresses, dates, descriptions, and status.",
    inputSchema: {
      type: "object",
      properties: {
        geo_id: { type: "string", description: "Geographic filter: state (e.g. 'TX'), zip code, city FIPS, or county FIPS" },
        permit_from: { type: "string", description: "Start date (YYYY-MM-DD)" },
        permit_to: { type: "string", description: "End date (YYYY-MM-DD)" },
        property_type: { type: "string", description: "Property type: residential, commercial, industrial, agricultural, office" },
        permit_tags: {
          type: "array", items: { type: "string" },
          description: "Permit types: solar, hvac, reroof, room_addition, kitchen_remodel, bath_remodel, new_dwelling, pool_spa, utilities, window_door",
        },
        page: { type: "number", description: "Page number (default 1)" },
      },
      required: ["geo_id", "permit_from", "permit_to"],
    },
  },
  {
    name: "shovels_get_permit",
    description: "Get detailed information about a specific building permit by ID.",
    inputSchema: {
      type: "object",
      properties: {
        id: { type: "string", description: "Permit ID from search results" },
      },
      required: ["id"],
    },
  },
  {
    name: "shovels_search_contractors",
    description: "Search contractors by jurisdiction, date range, and specialization. Returns contractor profiles with license info, permit history, and ratings.",
    inputSchema: {
      type: "object",
      properties: {
        geo_id: { type: "string", description: "Geographic filter: state, zip, city FIPS, or county FIPS" },
        permit_from: { type: "string", description: "Start date for permit activity (YYYY-MM-DD)" },
        permit_to: { type: "string", description: "End date for permit activity (YYYY-MM-DD)" },
        permit_tags: {
          type: "array", items: { type: "string" },
          description: "Specialization filter: solar, hvac, reroof, new_dwelling, etc.",
        },
        page: { type: "number", description: "Page number (default 1)" },
      },
      required: ["geo_id", "permit_from", "permit_to"],
    },
  },
  {
    name: "shovels_get_contractor",
    description: "Get detailed profile for a specific contractor by ID, including permit history and service area.",
    inputSchema: {
      type: "object",
      properties: {
        id: { type: "string", description: "Contractor ID from search results" },
      },
      required: ["id"],
    },
  },
  {
    name: "shovels_search_addresses",
    description: "Search and validate US addresses. Returns parcel data and associated permit history.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "Address search query" },
      },
      required: ["q"],
    },
  },
  {
    name: "shovels_get_geography",
    description: "Look up geographic identifiers (geo_id) for cities, counties, and zip codes. Use this to find the correct geo_id for permit/contractor searches.",
    inputSchema: {
      type: "object",
      properties: {
        q: { type: "string", description: "City name, county name, or zip code" },
        type: { type: "string", description: "Filter by type: city, county, zip" },
      },
      required: ["q"],
    },
  },
  {
    name: "shovels_list_permit_tags",
    description: "List all valid permit tag values for filtering searches.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "shovels_get_meta",
    description: "Get API metadata including coverage statistics and data freshness.",
    inputSchema: { type: "object", properties: {} },
  },
];

// ── Tool handlers ────────────────────────────────────────────────────
const handlers = {
  shovels_search_permits: (args) => fetchAPI("/permits/search", {
    geo_id: args.geo_id, permit_from: args.permit_from, permit_to: args.permit_to,
    property_type: args.property_type, permit_tags: args.permit_tags, page: args.page,
  }),
  shovels_get_permit: (args) => fetchAPI("/permits", { id: args.id }),
  shovels_search_contractors: (args) => fetchAPI("/contractors/search", {
    geo_id: args.geo_id, permit_from: args.permit_from, permit_to: args.permit_to,
    permit_tags: args.permit_tags, page: args.page,
  }),
  shovels_get_contractor: (args) => fetchAPI("/contractors", { id: args.id }),
  shovels_search_addresses: (args) => fetchAPI("/addresses", { q: args.q }),
  shovels_get_geography: (args) => fetchAPI("/geography", { q: args.q, type: args.type }),
  shovels_list_permit_tags: () => fetchAPI("/lists/permit_tags"),
  shovels_get_meta: () => fetchAPI("/meta"),
};

// ── Server ───────────────────────────────────────────────────────────
const server = new Server({ name: "shovels-mcp", version: "0.1.0" }, {
  capabilities: { tools: {} },
});

server.setRequestHandler({ method: "tools/list" }, async () => ({ tools: TOOLS }));

server.setRequestHandler({ method: "tools/call" }, async (request) => {
  const { name, arguments: args } = request.params;
  const handler = handlers[name];
  if (!handler) return { content: [{ type: "text", text: `Unknown tool: ${name}` }], isError: true };
  try {
    const result = await handler(args || {});
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  } catch (err) {
    return { content: [{ type: "text", text: `Error: ${err.message}` }], isError: true };
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
