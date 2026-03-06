#!/usr/bin/env node
// Plaid MCP server — banking accounts, transactions, balances, and identity.
// Requires PLAID_CLIENT_ID + PLAID_SECRET env vars. Optionally PLAID_ENV (sandbox|development|production).

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { PlaidApi, PlaidEnvironments, Configuration } from "plaid";
import { createHash } from "crypto";

const CLIENT_ID = process.env.PLAID_CLIENT_ID || "";
const SECRET = process.env.PLAID_SECRET || "";
const ENV = process.env.PLAID_ENV || "sandbox";

function getClient() {
  if (!CLIENT_ID || !SECRET) {
    throw new Error("PLAID_CLIENT_ID and PLAID_SECRET must be set. Get them at https://dashboard.plaid.com");
  }
  const config = new Configuration({
    basePath: PlaidEnvironments[ENV] || PlaidEnvironments.sandbox,
    baseOptions: {
      headers: {
        "PLAID-CLIENT-ID": CLIENT_ID,
        "PLAID-SECRET": SECRET,
      },
    },
  });
  return new PlaidApi(config);
}

let client = null;
function plaid() {
  if (!client) client = getClient();
  return client;
}

// ── Rate limiting ────────────────────────────────────────────────────
let timestamps = [];
const MAX_RPS = 5;
const WINDOW = 1000;

function checkRate() {
  const now = Date.now();
  timestamps = timestamps.filter(t => now - t < WINDOW);
  if (timestamps.length >= MAX_RPS) throw new Error("Rate limit: max 5 req/s");
  timestamps.push(now);
}

// ── Cache ────────────────────────────────────────────────────────────
const cache = new Map();
const CACHE_TTL = 5 * 60 * 1000;
const MAX_CACHE = 100;

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

// ── Tool definitions ─────────────────────────────────────────────────
const TOOLS = [
  {
    name: "plaid_create_link_token",
    description: "Create a Plaid Link token for connecting a new bank account. Returns a link_token to initialize Plaid Link in the frontend.",
    inputSchema: {
      type: "object",
      properties: {
        user_id: { type: "string", description: "Unique user identifier for this Link session" },
        products: {
          type: "array", items: { type: "string" },
          description: "Plaid products to enable: transactions, auth, identity, assets, investments, liabilities (default: [transactions])",
        },
        country_codes: {
          type: "array", items: { type: "string" },
          description: "Country codes (default: [US])",
        },
      },
      required: ["user_id"],
    },
  },
  {
    name: "plaid_exchange_public_token",
    description: "Exchange a public_token from Plaid Link for a permanent access_token. Call this after the user completes the Link flow.",
    inputSchema: {
      type: "object",
      properties: {
        public_token: { type: "string", description: "Public token from Plaid Link onSuccess callback" },
      },
      required: ["public_token"],
    },
  },
  {
    name: "plaid_get_accounts",
    description: "List all accounts (checking, savings, credit, loan, investment) for a connected institution.",
    inputSchema: {
      type: "object",
      properties: {
        access_token: { type: "string", description: "Plaid access token for the institution" },
      },
      required: ["access_token"],
    },
  },
  {
    name: "plaid_get_balances",
    description: "Get real-time balance information for all accounts at a connected institution.",
    inputSchema: {
      type: "object",
      properties: {
        access_token: { type: "string", description: "Plaid access token for the institution" },
        account_ids: {
          type: "array", items: { type: "string" },
          description: "Optional: filter to specific account IDs",
        },
      },
      required: ["access_token"],
    },
  },
  {
    name: "plaid_sync_transactions",
    description: "Incrementally sync transactions using the /transactions/sync endpoint. Returns added, modified, and removed transactions since the last cursor.",
    inputSchema: {
      type: "object",
      properties: {
        access_token: { type: "string", description: "Plaid access token for the institution" },
        cursor: { type: "string", description: "Cursor from previous sync call (omit for initial sync)" },
        count: { type: "number", description: "Max transactions per page (default 100, max 500)" },
      },
      required: ["access_token"],
    },
  },
  {
    name: "plaid_get_transactions",
    description: "Get transactions for a date range. Use plaid_sync_transactions for incremental updates; this is for historical pulls.",
    inputSchema: {
      type: "object",
      properties: {
        access_token: { type: "string", description: "Plaid access token for the institution" },
        start_date: { type: "string", description: "Start date (YYYY-MM-DD)" },
        end_date: { type: "string", description: "End date (YYYY-MM-DD)" },
        account_ids: {
          type: "array", items: { type: "string" },
          description: "Optional: filter to specific account IDs",
        },
        offset: { type: "number", description: "Pagination offset (default 0)" },
        count: { type: "number", description: "Max results per page (default 100, max 500)" },
      },
      required: ["access_token", "start_date", "end_date"],
    },
  },
  {
    name: "plaid_get_identity",
    description: "Get account holder identity information (name, address, email, phone) from the institution.",
    inputSchema: {
      type: "object",
      properties: {
        access_token: { type: "string", description: "Plaid access token for the institution" },
      },
      required: ["access_token"],
    },
  },
  {
    name: "plaid_get_institution",
    description: "Look up details about a financial institution by ID.",
    inputSchema: {
      type: "object",
      properties: {
        institution_id: { type: "string", description: "Plaid institution ID (e.g. ins_1)" },
        country_codes: {
          type: "array", items: { type: "string" },
          description: "Country codes (default: [US])",
        },
      },
      required: ["institution_id"],
    },
  },
  {
    name: "plaid_search_institutions",
    description: "Search financial institutions by name.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Institution name to search for" },
        products: {
          type: "array", items: { type: "string" },
          description: "Filter to institutions supporting these products",
        },
        country_codes: {
          type: "array", items: { type: "string" },
          description: "Country codes (default: [US])",
        },
      },
      required: ["query"],
    },
  },
  {
    name: "plaid_get_categories",
    description: "Get the full list of Plaid transaction categories.",
    inputSchema: { type: "object", properties: {} },
  },
];

// ── Tool handlers ────────────────────────────────────────────────────
const handlers = {
  plaid_create_link_token: async (args) => {
    checkRate();
    const resp = await plaid().linkTokenCreate({
      user: { client_user_id: args.user_id },
      client_name: "Borg Agent",
      products: args.products || ["transactions"],
      country_codes: args.country_codes || ["US"],
      language: "en",
    });
    return resp.data;
  },

  plaid_exchange_public_token: async (args) => {
    checkRate();
    const resp = await plaid().itemPublicTokenExchange({
      public_token: args.public_token,
    });
    return resp.data;
  },

  plaid_get_accounts: async (args) => {
    checkRate();
    const key = `accounts:${createHash("sha256").update(args.access_token).digest("hex").slice(0, 16)}`;
    const hit = cached(key);
    if (hit) return hit;
    const resp = await plaid().accountsGet({
      access_token: args.access_token,
    });
    return setCache(key, resp.data);
  },

  plaid_get_balances: async (args) => {
    checkRate();
    const opts = { access_token: args.access_token };
    if (args.account_ids?.length) {
      opts.options = { account_ids: args.account_ids };
    }
    const resp = await plaid().accountsBalanceGet(opts);
    return resp.data;
  },

  plaid_sync_transactions: async (args) => {
    checkRate();
    const opts = { access_token: args.access_token };
    if (args.cursor) opts.cursor = args.cursor;
    if (args.count) opts.count = args.count;
    const resp = await plaid().transactionsSync(opts);
    return resp.data;
  },

  plaid_get_transactions: async (args) => {
    checkRate();
    const opts = {
      access_token: args.access_token,
      start_date: args.start_date,
      end_date: args.end_date,
    };
    const options = {};
    if (args.account_ids?.length) options.account_ids = args.account_ids;
    if (args.offset) options.offset = args.offset;
    if (args.count) options.count = args.count;
    if (Object.keys(options).length) opts.options = options;
    const resp = await plaid().transactionsGet(opts);
    return resp.data;
  },

  plaid_get_identity: async (args) => {
    checkRate();
    const key = `identity:${createHash("sha256").update(args.access_token).digest("hex").slice(0, 16)}`;
    const hit = cached(key);
    if (hit) return hit;
    const resp = await plaid().identityGet({
      access_token: args.access_token,
    });
    return setCache(key, resp.data);
  },

  plaid_get_institution: async (args) => {
    checkRate();
    const key = `inst:${args.institution_id}`;
    const hit = cached(key);
    if (hit) return hit;
    const resp = await plaid().institutionsGetById({
      institution_id: args.institution_id,
      country_codes: args.country_codes || ["US"],
    });
    return setCache(key, resp.data);
  },

  plaid_search_institutions: async (args) => {
    checkRate();
    const resp = await plaid().institutionsSearch({
      query: args.query,
      products: args.products || ["transactions"],
      country_codes: args.country_codes || ["US"],
    });
    return resp.data;
  },

  plaid_get_categories: async () => {
    checkRate();
    const hit = cached("categories");
    if (hit) return hit;
    const resp = await plaid().categoriesGet({});
    return setCache("categories", resp.data);
  },
};

// ── Server ───────────────────────────────────────────────────────────
const server = new Server({ name: "plaid-mcp", version: "0.1.0" }, {
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
    const msg = err?.response?.data ? JSON.stringify(err.response.data) : err.message;
    return { content: [{ type: "text", text: `Error: ${msg}` }], isError: true };
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
