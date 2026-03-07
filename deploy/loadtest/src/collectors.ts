import type { GeneratedDoc, GroundTruth, SearchTestCase } from "./types";

const UA = "Borg-Loadtest/1.0 (loadtest@borg.legal)";
const COURTLISTENER = "https://www.courtlistener.com/api/rest/v4";
const EDGAR_SEARCH = "https://efts.sec.gov/LATEST";
const UK_LEG = "https://www.legislation.gov.uk";
const USPTO = "https://developer.uspto.gov/ibd-api/v1";
const CL_TOKEN = process.env.COURTLISTENER_TOKEN || "";

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function fetchJSON(url: string, headers?: Record<string, string>, retries = 2): Promise<any> {
  for (let i = 0; i <= retries; i++) {
    try {
      const resp = await fetch(url, { headers: { "User-Agent": UA, ...headers } });
      if (resp.status === 429) {
        const wait = Math.pow(2, i + 1) * 1000;
        console.log(`  rate limited, waiting ${wait / 1000}s...`);
        await delay(wait);
        continue;
      }
      if (!resp.ok) throw new Error(`${resp.status}: ${await resp.text().then(t => t.slice(0, 200))}`);
      return resp.json();
    } catch (e) {
      if (i === retries) throw e;
      await delay(1000);
    }
  }
}

async function fetchText(url: string, headers?: Record<string, string>): Promise<string> {
  try {
    const resp = await fetch(url, { headers: { "User-Agent": UA, ...headers } });
    if (!resp.ok) return "";
    let text = await resp.text();
    if (text.length > 30000) text = text.slice(0, 30000);
    return text;
  } catch {
    return "";
  }
}

function stripHtml(html: string): string {
  return html
    .replace(/<script[^>]*>[\s\S]*?<\/script>/gi, "")
    .replace(/<style[^>]*>[\s\S]*?<\/style>/gi, "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/p>/gi, "\n\n")
    .replace(/<\/div>/gi, "\n")
    .replace(/<\/h[1-6]>/gi, "\n\n")
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#\d+;/g, " ")
    .replace(/ {2,}/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

const clHeaders = CL_TOKEN ? { Authorization: `Token ${CL_TOKEN}` } : undefined;

// ── CourtListener ──────────────────────────────────────────────────

const CL_TOPICS = [
  { query: "patent infringement claim construction", court: "cafc", label: "patent" },
  { query: "securities fraud 10b-5 scienter", court: "", label: "securities" },
  { query: "breach of fiduciary duty shareholder derivative", court: "ded", label: "fiduciary" },
  { query: "employment discrimination Title VII", court: "", label: "employment" },
  { query: "antitrust Sherman Act monopoly", court: "", label: "antitrust" },
  { query: "summary judgment standard genuine dispute", court: "", label: "summary_judgment" },
  { query: "preliminary injunction irreparable harm", court: "", label: "injunction" },
  { query: "class certification Rule 23 numerosity", court: "", label: "class_action" },
  { query: "trade secret misappropriation DTSA", court: "", label: "trade_secret" },
  { query: "ERISA fiduciary duty plan administrator", court: "", label: "erisa" },
  { query: "merger appraisal fair value Delaware", court: "", label: "appraisal" },
  { query: "contract breach damages consequential", court: "", label: "contract" },
  { query: "bankruptcy reorganization chapter 11 plan", court: "", label: "bankruptcy" },
  { query: "copyright fair use transformative", court: "", label: "copyright" },
  { query: "due process substantive fundamental right", court: "", label: "due_process" },
  { query: "arbitration agreement enforceability FAA", court: "", label: "arbitration" },
  { query: "standing injury in fact causation", court: "", label: "standing" },
  { query: "qualified immunity clearly established", court: "", label: "qualified_immunity" },
  { query: "environmental NEPA impact statement", court: "", label: "environmental" },
  { query: "first amendment free speech content neutral", court: "", label: "first_amendment" },
  { query: "negligence proximate cause duty breach", court: "", label: "negligence" },
  { query: "RICO enterprise pattern racketeering", court: "", label: "rico" },
  { query: "habeas corpus ineffective assistance counsel", court: "", label: "habeas" },
  { query: "ADA disability reasonable accommodation", court: "", label: "ada" },
  { query: "immigration asylum withholding removal", court: "", label: "immigration" },
  { query: "tax deficiency penalty reasonable cause", court: "", label: "tax" },
  { query: "insurance coverage bad faith denial", court: "", label: "insurance" },
  { query: "product liability design defect strict", court: "", label: "product_liability" },
  { query: "real property easement adverse possession", court: "", label: "property" },
  { query: "medical malpractice standard of care", court: "", label: "med_mal" },
];

function courtToJurisdiction(courtId: string): string {
  if (courtId.startsWith("de") || courtId === "dech") return "Delaware";
  if (courtId.startsWith("ny") || courtId === "nyed" || courtId === "nysd") return "New York";
  if (courtId.startsWith("ca") || courtId === "cand" || courtId === "cacd") return "California";
  if (courtId.startsWith("tx") || courtId === "txed" || courtId === "txsd") return "Texas";
  if (courtId === "scotus") return "Federal";
  if (courtId === "cafc") return "Federal";
  if (/^ca\d+$/.test(courtId)) return "Federal";
  return "Federal";
}

async function collectCourtListenerOpinions(perTopic: number): Promise<GeneratedDoc[]> {
  const docs: GeneratedDoc[] = [];
  const seenIds = new Set<number>();

  for (const topic of CL_TOPICS) {
    console.log(`  CourtListener: "${topic.label}"`);
    try {
      const params = new URLSearchParams({
        q: topic.query,
        type: "o",
        page_size: String(Math.min(perTopic * 3, 20)),
        ordering: "score desc",
      });
      if (topic.court) params.set("court", topic.court);

      const data = await fetchJSON(`${COURTLISTENER}/search/?${params}`, clHeaders);
      const results = data.results || [];

      let collected = 0;
      for (const r of results) {
        if (collected >= perTopic) break;
        if (seenIds.has(r.cluster_id)) continue;
        seenIds.add(r.cluster_id);

        let text = "";

        if (CL_TOKEN) {
          await delay(500);
          try {
            const cluster = await fetchJSON(
              `${COURTLISTENER}/clusters/${r.cluster_id}/`,
              clHeaders,
            );
            const opinions = cluster.sub_opinions || cluster.opinions || [];
            for (const op of opinions) {
              if (op.plain_text && op.plain_text.length > 200) { text = op.plain_text; break; }
              if (op.html_with_citations) { text = stripHtml(op.html_with_citations); break; }
              if (op.html) { text = stripHtml(op.html); break; }
            }
            if (text.length > 30000) text = text.slice(0, 30000);
          } catch { /* fall back to snippet */ }
        }

        if (!text || text.length < 200) {
          const snippets = (r.opinions || [])
            .map((op: any) => op.snippet || "")
            .filter(Boolean)
            .map(stripHtml);
          text = snippets.join("\n\n");
        }
        if (text.length < 50) continue;
        if (text.length > 30000) text = text.slice(0, 30000);

        const citations = r.citation || [];
        const entities = [r.caseName];
        if (r.judge) entities.push(r.judge);

        docs.push({
          file_name: `cl_${topic.label}_${r.cluster_id}.md`,
          body: `${r.caseName}\n${citations.join(", ")}\n${r.court}\nDocket: ${r.docketNumber || "N/A"}\nFiled: ${r.dateFiled || "N/A"}\nJudge: ${r.judge || "N/A"}\n\n${text}`,
          doc_type: "filing",
          jurisdiction: courtToJurisdiction(r.court_id || ""),
          privileged: false,
          ground_truth: {
            unique_markers: [`cl-${r.cluster_id}`, r.docketNumber || "", ...citations].filter(Boolean),
            topics: [topic.label, topic.query],
            citations,
            entities,
            tags: ["courtlistener", topic.label, r.court_id || ""],
          },
        });
        collected++;
      }
      console.log(`    ${collected} opinions`);
    } catch (e) {
      console.log(`    failed: ${e}`);
    }
    await delay(300);
  }

  return docs;
}

// ── CourtListener Dockets ─────────────────────────────────────────

const CL_DOCKET_TOPICS = [
  { query: "class action settlement", label: "class_settlement" },
  { query: "patent litigation Markman", label: "patent_docket" },
  { query: "SEC enforcement fraud", label: "sec_docket" },
  { query: "merger acquisition antitrust", label: "merger_docket" },
  { query: "FOIA government records", label: "foia_docket" },
  { query: "trademark infringement Lanham", label: "trademark_docket" },
  { query: "whistleblower retaliation", label: "whistleblower_docket" },
  { query: "data breach privacy", label: "privacy_docket" },
  { query: "environmental cleanup Superfund", label: "env_docket" },
  { query: "labor union collective bargaining", label: "labor_docket" },
];

async function collectCourtListenerDockets(perTopic: number): Promise<GeneratedDoc[]> {
  const docs: GeneratedDoc[] = [];
  const seenIds = new Set<number>();

  for (const topic of CL_DOCKET_TOPICS) {
    console.log(`  CL Dockets: "${topic.label}"`);
    try {
      const params = new URLSearchParams({
        q: topic.query,
        type: "d",
        page_size: String(Math.min(perTopic * 2, 20)),
        ordering: "score desc",
      });

      const data = await fetchJSON(`${COURTLISTENER}/search/?${params}`, clHeaders);
      const results = data.results || [];

      let collected = 0;
      for (const r of results) {
        if (collected >= perTopic) break;
        if (seenIds.has(r.docket_id)) continue;
        seenIds.add(r.docket_id);

        const caseName = r.caseName || "Unknown Case";
        const court = r.court || "";
        const docketNum = r.docketNumber || "";
        const snippet = r.snippet ? stripHtml(r.snippet) : "";
        const nature = r.suitNature || "";

        const body = `${caseName}\n${court}\nDocket: ${docketNum}\nNature of Suit: ${nature}\nFiled: ${r.dateFiled || "N/A"}\n\n${snippet}`;
        if (body.length < 100) continue;

        docs.push({
          file_name: `cl_docket_${topic.label}_${r.docket_id}.md`,
          body,
          doc_type: "filing",
          jurisdiction: courtToJurisdiction(r.court_id || ""),
          privileged: false,
          ground_truth: {
            unique_markers: [`docket-${r.docket_id}`, docketNum].filter(Boolean),
            topics: [topic.label, topic.query],
            citations: [],
            entities: [caseName],
            tags: ["courtlistener", "docket", topic.label],
          },
        });
        collected++;
      }
      console.log(`    ${collected} dockets`);
    } catch (e) {
      console.log(`    failed: ${e}`);
    }
    await delay(300);
  }

  return docs;
}

// ── SEC EDGAR ──────────────────────────────────────────────────────

const EDGAR_QUERIES = [
  { query: "merger agreement", forms: "8-K", label: "merger" },
  { query: "breach of fiduciary duty", forms: "10-K", label: "fiduciary_risk" },
  { query: "force majeure pandemic", forms: "10-K,10-Q", label: "force_majeure" },
  { query: "intellectual property infringement", forms: "10-K", label: "ip_risk" },
  { query: "cybersecurity incident data breach", forms: "8-K", label: "cyber" },
  { query: "antitrust investigation", forms: "10-K,8-K", label: "antitrust" },
  { query: "environmental remediation liability", forms: "10-K", label: "environmental" },
  { query: "whistleblower complaint retaliation", forms: "10-K,8-K", label: "whistleblower" },
  { query: "settlement agreement litigation", forms: "8-K", label: "settlement" },
  { query: "SEC investigation enforcement action", forms: "10-K,8-K", label: "sec_enforcement" },
  { query: "goodwill impairment writedown", forms: "10-K,10-Q", label: "impairment" },
  { query: "material weakness internal control", forms: "10-K", label: "internal_control" },
  { query: "related party transaction conflict", forms: "10-K,DEF 14A", label: "related_party" },
  { query: "going concern substantial doubt", forms: "10-K", label: "going_concern" },
  { query: "stock option grant backdating", forms: "10-K,8-K", label: "stock_option" },
  { query: "supply chain disruption shortage", forms: "10-K,10-Q", label: "supply_chain" },
  { query: "climate change risk emission", forms: "10-K", label: "climate_risk" },
  { query: "executive compensation severance", forms: "DEF 14A", label: "exec_comp" },
  { query: "derivative lawsuit shareholder demand", forms: "10-K,8-K", label: "derivative" },
  { query: "restatement accounting error", forms: "10-K,8-K", label: "restatement" },
];

async function fetchEdgarFilingText(accession: string, cik: string): Promise<string> {
  const parts = accession.split(":");
  if (parts.length !== 2) return "";

  const file = parts[1];
  const cikNum = cik.replace(/^0+/, "");
  const accNum = parts[0].replace(/-/g, "");

  const url = `https://www.sec.gov/Archives/edgar/data/${cikNum}/${accNum}/${file}`;
  try {
    const resp = await fetch(url, { headers: { "User-Agent": UA } });
    if (!resp.ok) return "";
    const html = await resp.text();
    let text = stripHtml(html);
    if (text.length > 30000) text = text.slice(0, 30000);
    return text;
  } catch {
    return "";
  }
}

function stateToJurisdiction(state: string): string {
  const map: Record<string, string> = {
    DE: "Delaware", NY: "New York", CA: "California", TX: "Texas",
    IL: "Illinois", MA: "Massachusetts", FL: "Florida", PA: "Pennsylvania",
    VA: "Virginia", DC: "District of Columbia", NV: "Nevada", GA: "Georgia",
    OH: "Ohio", WA: "Washington", CO: "Colorado", MN: "Minnesota",
    CT: "Connecticut", NJ: "New Jersey", MD: "Maryland", NC: "North Carolina",
  };
  return map[state] || state || "Federal";
}

async function collectEdgarFilings(perTopic: number): Promise<GeneratedDoc[]> {
  const docs: GeneratedDoc[] = [];
  const seenIds = new Set<string>();

  for (const topic of EDGAR_QUERIES) {
    console.log(`  EDGAR: "${topic.label}"`);
    try {
      const params = new URLSearchParams({
        q: `"${topic.query}"`,
        forms: topic.forms,
        dateRange: "custom",
        startdt: "2020-01-01",
      });

      const data = await fetchJSON(`${EDGAR_SEARCH}/search-index?${params}`);
      const hits = (data.hits?.hits || []).slice(0, perTopic * 2);

      let collected = 0;
      for (const hit of hits) {
        if (collected >= perTopic) break;
        if (seenIds.has(hit._id)) continue;
        seenIds.add(hit._id);

        const src = hit._source;
        const entityName = src.display_names?.[0] || src.entity_name || "Unknown";
        const form = src.root_forms?.[0] || "filing";
        const cik = src.ciks?.[0] || "";
        const state = src.biz_states?.[0] || "";

        await delay(200);
        const text = await fetchEdgarFilingText(hit._id, cik);
        if (text.length < 500) continue;

        docs.push({
          file_name: `edgar_${topic.label}_${cik.replace(/^0+/, "")}_${collected}.md`,
          body: `${entityName}\nSEC Filing: ${form}\nCIK: ${cik}\nFiled: ${src.file_date || "N/A"}\nPeriod: ${src.period_ending || "N/A"}\n\n${text}`,
          doc_type: "document",
          jurisdiction: stateToJurisdiction(state),
          privileged: false,
          ground_truth: {
            unique_markers: [`edgar-${hit._id}`, cik],
            topics: [topic.label, topic.query, form],
            citations: [],
            entities: [entityName],
            tags: ["edgar", topic.label, form.toLowerCase()],
          },
        });
        collected++;
      }
      console.log(`    ${collected} filings`);
    } catch (e) {
      console.log(`    failed: ${e}`);
    }
    await delay(300);
  }

  return docs;
}

// ── Federal Register ───────────────────────────────────────────────

const FEDREG_QUERIES = [
  { term: "cybersecurity", agencies: ["securities-and-exchange-commission"], label: "sec_cyber" },
  { term: "climate disclosure", agencies: ["securities-and-exchange-commission"], label: "sec_climate" },
  { term: "antitrust merger", agencies: ["federal-trade-commission"], label: "ftc_merger" },
  { term: "data privacy", agencies: ["federal-trade-commission"], label: "ftc_privacy" },
  { term: "emission standards", agencies: ["environmental-protection-agency"], label: "epa_emissions" },
  { term: "bank regulation capital", agencies: ["federal-reserve-system"], label: "fed_capital" },
  { term: "consumer protection", agencies: ["consumer-financial-protection-bureau"], label: "cfpb" },
  { term: "labor minimum wage", agencies: ["labor-department"], label: "dol_wage" },
  { term: "drug approval safety", agencies: ["food-and-drug-administration"], label: "fda_drug" },
  { term: "telecommunications spectrum", agencies: ["federal-communications-commission"], label: "fcc_telecom" },
  { term: "energy pipeline safety", agencies: ["energy-department"], label: "doe_energy" },
  { term: "aviation safety regulation", agencies: ["federal-aviation-administration"], label: "faa_safety" },
  { term: "export control sanctions", agencies: ["commerce-department"], label: "bis_export" },
  { term: "immigration enforcement", agencies: ["homeland-security-department"], label: "dhs_immigration" },
  { term: "housing discrimination fair", agencies: ["housing-and-urban-development-department"], label: "hud_housing" },
];

async function collectFederalRegister(perTopic: number): Promise<GeneratedDoc[]> {
  const docs: GeneratedDoc[] = [];

  for (const topic of FEDREG_QUERIES) {
    console.log(`  Federal Register: "${topic.label}"`);
    try {
      const params = new URLSearchParams({
        "conditions[term]": topic.term,
        per_page: String(Math.min(perTopic, 20)),
        order: "relevance",
      });
      for (const agency of topic.agencies) {
        params.append("conditions[agencies][]", agency);
      }

      const data = await fetchJSON(
        `https://www.federalregister.gov/api/v1/documents.json?${params}`,
      );
      const results = data.results || [];

      let collected = 0;
      for (const r of results) {
        const text = r.raw_text_url
          ? await fetchText(r.raw_text_url)
          : r.abstract || r.body || "";
        if (text.length < 300) continue;

        const truncated = text.length > 20000 ? text.slice(0, 20000) : text;

        docs.push({
          file_name: `fedreg_${topic.label}_${r.document_number || docs.length}.md`,
          body: `${r.title || "Federal Register Document"}\nDocument: ${r.document_number || "N/A"}\nType: ${r.type || "N/A"}\nAgency: ${(r.agencies || []).map((a: any) => a.name).join(", ")}\nPublished: ${r.publication_date || "N/A"}\n\n${truncated}`,
          doc_type: "statute",
          jurisdiction: "Federal",
          privileged: false,
          ground_truth: {
            unique_markers: [r.document_number, `fedreg-${r.document_number}`].filter(Boolean),
            topics: [topic.label, topic.term],
            citations: [r.citation || ""].filter(Boolean),
            entities: (r.agencies || []).map((a: any) => a.name),
            tags: ["federal_register", topic.label, r.type || ""],
          },
        });
        collected++;
      }
      console.log(`    ${collected} documents`);
    } catch (e) {
      console.log(`    failed: ${e}`);
    }
    await delay(300);
  }

  return docs;
}

// ── UK Legislation ────────────────────────────────────────────────

const UK_TOPICS = [
  { query: "data protection", type: "ukpga", label: "uk_data_protection" },
  { query: "companies act", type: "ukpga", label: "uk_companies" },
  { query: "employment rights", type: "ukpga", label: "uk_employment" },
  { query: "financial services", type: "ukpga", label: "uk_financial" },
  { query: "human rights", type: "ukpga", label: "uk_human_rights" },
  { query: "consumer rights", type: "ukpga", label: "uk_consumer" },
  { query: "insolvency", type: "ukpga", label: "uk_insolvency" },
  { query: "competition", type: "ukpga", label: "uk_competition" },
  { query: "intellectual property", type: "ukpga", label: "uk_ip" },
  { query: "environment", type: "ukpga", label: "uk_environment" },
  { query: "health safety", type: "uksi", label: "uk_health_safety" },
  { query: "money laundering", type: "uksi", label: "uk_aml" },
  { query: "sanctions regulations", type: "uksi", label: "uk_sanctions" },
  { query: "building regulations", type: "uksi", label: "uk_building" },
  { query: "immigration rules", type: "uksi", label: "uk_immigration" },
];

async function collectUKLegislation(perTopic: number): Promise<GeneratedDoc[]> {
  const docs: GeneratedDoc[] = [];
  const seenIds = new Set<string>();

  for (const topic of UK_TOPICS) {
    console.log(`  UK Legislation: "${topic.label}"`);
    try {
      // UK Legislation returns Atom XML — fetch and parse it
      const url = `${UK_LEG}/${topic.type}?text=${encodeURIComponent(topic.query)}`;
      const resp = await fetch(url, {
        headers: { "User-Agent": UA, Accept: "application/atom+xml" },
        redirect: "follow",
      });
      if (!resp.ok) { console.log(`    ${resp.status}`); continue; }
      const xml = await resp.text();

      // Parse Atom entries with regex (avoiding XML parser dep)
      const entryBlocks = xml.split(/<entry>/g).slice(1);

      let collected = 0;
      for (const block of entryBlocks) {
        if (collected >= perTopic) break;

        const title = block.match(/<title>([^<]+)<\/title>/)?.[1] || "";
        const id = block.match(/<id>([^<]+)<\/id>/)?.[1] || "";
        const summary = block.match(/<summary>([^<]+)<\/summary>/)?.[1] || "";
        const year = block.match(/Year Value="(\d+)"/)?.[1] || "";
        const number = block.match(/Number Value="(\d+)"/)?.[1] || "";
        const htmLink = block.match(/href="([^"]+\/data\.htm)"/)?.[1] || "";

        if (!title || seenIds.has(id)) continue;
        seenIds.add(id);

        let text = "";
        if (htmLink) {
          await delay(500);
          const fetched = await fetchText(htmLink);
          if (fetched) text = stripHtml(fetched);
        }

        if (text.length < 200) text = summary || title;
        if (text.length < 50) continue;
        if (text.length > 25000) text = text.slice(0, 25000);

        docs.push({
          file_name: `uk_${topic.label}_${year}_${number || collected}.md`,
          body: `${title}\nType: ${topic.type.toUpperCase()}\nYear: ${year}\nNumber: ${number}\n\n${text}`,
          doc_type: "statute",
          jurisdiction: "United Kingdom",
          privileged: false,
          ground_truth: {
            unique_markers: [`uk-${topic.type}-${year}-${number}`].filter(Boolean),
            topics: [topic.label, topic.query],
            citations: [],
            entities: [],
            tags: ["uk_legislation", topic.label, topic.type],
          },
        });
        collected++;
      }
      console.log(`    ${collected} statutes`);
    } catch (e) {
      console.log(`    failed: ${e}`);
    }
    await delay(500);
  }

  return docs;
}

// ── EUR-Lex ───────────────────────────────────────────────────────

const EURLEX_TOPICS = [
  { text: "data protection", type: "REG", label: "eu_data_protection" },
  { text: "competition", type: "REG", label: "eu_competition" },
  { text: "financial", type: "DIR", label: "eu_financial" },
  { text: "environment", type: "DIR", label: "eu_environment" },
  { text: "consumer", type: "DIR", label: "eu_consumer" },
  { text: "digital", type: "REG", label: "eu_digital" },
  { text: "trade", type: "REG", label: "eu_trade" },
  { text: "energy", type: "DIR", label: "eu_energy" },
  { text: "pharmaceutical", type: "REG", label: "eu_pharma" },
  { text: "securities", type: "REG", label: "eu_securities" },
  { text: "anti-money laundering", type: "DIR", label: "eu_aml" },
  { text: "artificial intelligence", type: "REG", label: "eu_ai" },
  { text: "sanctions", type: "REG", label: "eu_sanctions" },
  { text: "employment", type: "DIR", label: "eu_employment" },
  { text: "intellectual property", type: "REG", label: "eu_ip" },
];

async function collectEurLex(perTopic: number): Promise<GeneratedDoc[]> {
  const docs: GeneratedDoc[] = [];
  const seenCelex = new Set<string>();
  const typeMap: Record<string, string> = { REG: "regulation", DIR: "directive", DEC: "decision", JUDG: "judgment" };

  for (const topic of EURLEX_TOPICS) {
    console.log(`  EUR-Lex: "${topic.label}"`);
    try {
      const sparql = `PREFIX cdm: <http://publications.europa.eu/ontology/cdm#> SELECT DISTINCT ?celex ?title WHERE { ?work cdm:resource_legal_id_celex ?celex . ?exp cdm:expression_belongs_to_work ?work . ?exp cdm:expression_uses_language <http://publications.europa.eu/resource/authority/language/ENG> . ?exp cdm:expression_title ?title . FILTER(CONTAINS(LCASE(STR(?title)), "${topic.text.toLowerCase()}")) } LIMIT ${perTopic * 2}`;

      const resp = await fetch("https://publications.europa.eu/webapi/rdf/sparql", {
        method: "POST",
        headers: {
          "User-Agent": UA,
          Accept: "application/sparql-results+json",
          "Content-Type": "application/x-www-form-urlencoded",
        },
        body: `query=${encodeURIComponent(sparql)}`,
      });

      if (!resp.ok) {
        console.log(`    SPARQL error: ${resp.status}`);
        continue;
      }

      const data = await resp.json();
      const bindings = data.results?.bindings || [];

      let collected = 0;
      for (const b of bindings) {
        if (collected >= perTopic) break;
        const celex = b.celex?.value || "";
        const title = b.title?.value || "";
        const date = "";
        if (!celex || seenCelex.has(celex)) continue;
        seenCelex.add(celex);

        // Fetch full text
        await delay(500);
        const htmlText = await fetchText(
          `https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:${celex}`,
        );
        let text = htmlText ? stripHtml(htmlText) : title;
        if (text.length > 25000) text = text.slice(0, 25000);
        if (text.length < 100) continue;

        docs.push({
          file_name: `eurlex_${topic.label}_${celex}.md`,
          body: `${title}\nCELEX: ${celex}\nDate: ${date}\nType: ${topic.type}\n\n${text}`,
          doc_type: "statute",
          jurisdiction: "European Union",
          privileged: false,
          ground_truth: {
            unique_markers: [celex, `eurlex-${celex}`],
            topics: [topic.label, topic.text],
            citations: [celex],
            entities: [],
            tags: ["eurlex", topic.label, topic.type.toLowerCase()],
          },
        });
        collected++;
      }
      console.log(`    ${collected} documents`);
    } catch (e) {
      console.log(`    failed: ${e}`);
    }
    await delay(500);
  }

  return docs;
}

// ── USPTO Patents ─────────────────────────────────────────────────

const USPTO_QUERIES = [
  { query: "artificial intelligence machine learning", label: "ai_patent" },
  { query: "blockchain distributed ledger", label: "blockchain_patent" },
  { query: "pharmaceutical drug delivery", label: "pharma_patent" },
  { query: "semiconductor chip design", label: "semiconductor_patent" },
  { query: "autonomous vehicle navigation", label: "av_patent" },
  { query: "gene editing CRISPR", label: "biotech_patent" },
  { query: "battery energy storage lithium", label: "battery_patent" },
  { query: "quantum computing qubit", label: "quantum_patent" },
  { query: "5G wireless communication", label: "telecom_patent" },
  { query: "medical device implant", label: "meddevice_patent" },
  { query: "solar panel photovoltaic", label: "solar_patent" },
  { query: "cybersecurity encryption", label: "security_patent" },
  { query: "natural language processing", label: "nlp_patent" },
  { query: "drone unmanned aerial", label: "drone_patent" },
  { query: "augmented reality display", label: "ar_patent" },
];

async function collectUSPTOPatents(perTopic: number): Promise<GeneratedDoc[]> {
  const docs: GeneratedDoc[] = [];
  const seenIds = new Set<string>();

  for (const topic of USPTO_QUERIES) {
    console.log(`  USPTO: "${topic.label}"`);
    try {
      const params = new URLSearchParams({
        searchText: topic.query,
        start: "0",
        rows: String(Math.min(perTopic * 2, 20)),
      });

      const data = await fetchJSON(`${USPTO}/application?${params}`);
      const results = data.results || data.response?.docs || [];

      let collected = 0;
      for (const r of results) {
        if (collected >= perTopic) break;
        const patentId = r.patentNumber || r.applicationNumber || r.publicationNumber || "";
        if (!patentId || seenIds.has(patentId)) continue;
        seenIds.add(patentId);

        const title = r.inventionTitle || r.title || "";
        const abstract_ = r.abstractText || r.abstract || "";
        const inventors = (r.inventorName || r.inventors || []).join?.(", ") || String(r.inventorName || "");
        const assignee = r.assigneeEntityName || r.assignee || "";
        const filingDate = r.filingDate || r.applicationFilingDate || "";

        const body = `${title}\nPatent: ${patentId}\nInventors: ${inventors}\nAssignee: ${assignee}\nFiled: ${filingDate}\n\n${abstract_}`;
        if (body.length < 100) continue;

        docs.push({
          file_name: `uspto_${topic.label}_${patentId}.md`,
          body,
          doc_type: "document",
          jurisdiction: "Federal",
          privileged: false,
          ground_truth: {
            unique_markers: [patentId, `patent-${patentId}`],
            topics: [topic.label, topic.query],
            citations: [],
            entities: [assignee, ...inventors.split(", ")].filter(Boolean),
            tags: ["uspto", topic.label],
          },
        });
        collected++;
      }
      console.log(`    ${collected} patents`);
    } catch (e) {
      console.log(`    failed: ${e}`);
    }
    await delay(300);
  }

  return docs;
}

// ── Main collector ─────────────────────────────────────────────────

export interface CollectOptions {
  opinionsPerTopic: number;
  docketsPerTopic: number;
  edgarPerTopic: number;
  fedregPerTopic: number;
  ukPerTopic: number;
  eurlexPerTopic: number;
  usptoPerTopic: number;
  cachePath: string;
}

export async function collectRealCorpus(opts: CollectOptions): Promise<GeneratedDoc[]> {
  // Check cache
  try {
    const cached = await Bun.file(opts.cachePath).text();
    const parsed = JSON.parse(cached);
    if (Array.isArray(parsed) && parsed.length > 0) {
      console.log(`  loaded ${parsed.length} docs from cache (${opts.cachePath})`);
      return parsed;
    }
  } catch {
    // no cache
  }

  console.log("\n=== COLLECTING REAL DOCUMENTS ===\n");

  const allDocs: GeneratedDoc[] = [];

  const sources = [
    { name: "CourtListener opinions", fn: () => collectCourtListenerOpinions(opts.opinionsPerTopic) },
    { name: "CourtListener dockets", fn: () => collectCourtListenerDockets(opts.docketsPerTopic) },
    { name: "SEC EDGAR filings", fn: () => collectEdgarFilings(opts.edgarPerTopic) },
    { name: "Federal Register", fn: () => collectFederalRegister(opts.fedregPerTopic) },
    { name: "UK Legislation", fn: () => collectUKLegislation(opts.ukPerTopic) },
    { name: "EUR-Lex", fn: () => collectEurLex(opts.eurlexPerTopic) },
    { name: "USPTO Patents", fn: () => collectUSPTOPatents(opts.usptoPerTopic) },
  ];

  for (const source of sources) {
    console.log(`\n${source.name}:`);
    const docs = await source.fn();
    allDocs.push(...docs);
    console.log(`  subtotal: ${docs.length}\n`);
  }

  console.log(`=== TOTAL: ${allDocs.length} real documents collected ===\n`);

  await Bun.write(opts.cachePath, JSON.stringify(allDocs, null, 2));
  console.log(`  cached to ${opts.cachePath}`);

  return allDocs;
}

// ── Real corpus test cases ─────────────────────────────────────────

export function buildRealTestCases(docs: GeneratedDoc[]): SearchTestCase[] {
  const tests: SearchTestCase[] = [];

  const byTag: Record<string, GeneratedDoc[]> = {};
  for (const doc of docs) {
    for (const tag of doc.ground_truth.tags) {
      if (!byTag[tag]) byTag[tag] = [];
      byTag[tag].push(doc);
    }
  }

  const clDocs = byTag["courtlistener"] || [];
  const edgarDocs = byTag["edgar"] || [];
  const fedregDocs = byTag["federal_register"] || [];
  const ukDocs = byTag["uk_legislation"] || [];
  const euDocs = byTag["eurlex"] || [];
  const patentDocs = byTag["uspto"] || [];

  // ── Citation / ID lookup (exact) ───────────────────────────────

  for (const doc of clDocs.slice(0, 8)) {
    const cites = doc.ground_truth.citations.filter((c) => c.match(/\d+\s+\w/));
    if (cites.length > 0) {
      tests.push({
        name: `Real-exact: citation "${cites[0].slice(0, 30)}"`,
        category: "exact_term",
        query: cites[0],
        expected_hits: [doc.file_name],
        min_recall: 1.0,
        limit: 20,
      });
    }
  }

  // Case name lookup
  for (const doc of clDocs.slice(0, 5)) {
    const caseName = doc.ground_truth.entities[0];
    if (caseName && caseName.length > 5) {
      tests.push({
        name: `Real-exact: case "${caseName.slice(0, 40)}"`,
        category: "exact_term",
        query: caseName,
        expected_hits: [doc.file_name],
        min_recall: 1.0,
        top_rank: 5,
        limit: 20,
      });
    }
  }

  // EDGAR entity lookup
  for (const doc of edgarDocs.slice(0, 5)) {
    const entity = doc.ground_truth.entities[0];
    if (entity && entity.length > 3) {
      const shortName = entity.split("(")[0].trim().slice(0, 50);
      tests.push({
        name: `Real-exact: entity "${shortName}"`,
        category: "exact_term",
        query: shortName,
        expected_hits: [doc.file_name],
        min_recall: 1.0,
        limit: 20,
      });
    }
  }

  // CELEX lookup
  for (const doc of euDocs.slice(0, 3)) {
    const celex = doc.ground_truth.citations[0];
    if (celex) {
      tests.push({
        name: `Real-exact: CELEX "${celex}"`,
        category: "exact_term",
        query: celex,
        expected_hits: [doc.file_name],
        min_recall: 1.0,
        limit: 20,
      });
    }
  }

  // Patent number lookup
  for (const doc of patentDocs.slice(0, 3)) {
    const patNum = doc.ground_truth.unique_markers[0];
    if (patNum) {
      tests.push({
        name: `Real-exact: patent "${patNum}"`,
        category: "exact_term",
        query: patNum,
        expected_hits: [doc.file_name],
        min_recall: 1.0,
        limit: 20,
      });
    }
  }

  // ── Topic-based retrieval ──────────────────────────────────────

  const topicTests: Array<{ tag: string; query: string; label: string }> = [
    { tag: "patent", query: "patent infringement claim construction prior art", label: "patent cases" },
    { tag: "securities", query: "securities fraud material misrepresentation 10b-5 scienter", label: "securities fraud" },
    { tag: "employment", query: "employment discrimination Title VII hostile work environment", label: "employment discrimination" },
    { tag: "bankruptcy", query: "chapter 11 reorganization creditor plan confirmation", label: "bankruptcy" },
    { tag: "copyright", query: "copyright infringement fair use transformative work", label: "copyright" },
    { tag: "merger", query: "merger agreement acquisition consideration closing", label: "mergers" },
    { tag: "cyber", query: "cybersecurity data breach incident notification", label: "cybersecurity" },
    { tag: "climate_risk", query: "climate change emission risk disclosure", label: "climate risk" },
    { tag: "ai_patent", query: "artificial intelligence machine learning neural network", label: "AI patents" },
    { tag: "eu_data_protection", query: "data protection privacy GDPR personal data", label: "EU data protection" },
    { tag: "uk_companies", query: "companies act directors duties shareholder", label: "UK company law" },
  ];

  for (const t of topicTests) {
    const tagDocs = byTag[t.tag] || [];
    if (tagDocs.length > 0) {
      tests.push({
        name: `Real-topic: ${t.label}`,
        category: "agent_realistic",
        query: t.query,
        expected_hits: tagDocs.map((d) => d.file_name),
        min_recall: Math.min(1.0, 2 / tagDocs.length),
        limit: 20,
      });
    }
  }

  // ── Cross-jurisdiction queries ─────────────────────────────────

  if (ukDocs.length > 0 && euDocs.length > 0) {
    tests.push({
      name: "Real-semantic: data protection laws (cross-jurisdiction)",
      category: "semantic",
      query: "personal data protection privacy rights regulations",
      expected_hits: [],
      min_results: 2,
      limit: 20,
    });
  }

  // ── Chunk precision on real docs ───────────────────────────────

  if (clDocs.length > 5) {
    tests.push({
      name: "Real-chunk: court opinions about injunctions",
      category: "chunk_precision",
      query: "preliminary injunction irreparable harm likelihood success merits",
      expected_hits: [],
      min_results: 1,
      expected_chunk_terms: ["injunction", "irreparable", "harm"],
      min_chunk_precision: 0.3,
      limit: 10,
    });
  }

  if (edgarDocs.length > 5) {
    tests.push({
      name: "Real-chunk: SEC filings about cybersecurity",
      category: "chunk_precision",
      query: "cybersecurity data breach incident response notification",
      expected_hits: [],
      min_results: 1,
      expected_chunk_terms: ["cyber", "breach", "data", "security"],
      min_chunk_precision: 0.3,
      limit: 10,
    });
  }

  if (patentDocs.length > 5) {
    tests.push({
      name: "Real-chunk: patent abstracts about machine learning",
      category: "chunk_precision",
      query: "machine learning model training neural network inference",
      expected_hits: [],
      min_results: 1,
      expected_chunk_terms: ["learning", "model", "neural", "training"],
      min_chunk_precision: 0.3,
      limit: 10,
    });
  }

  // ── Semantic paraphrase queries ────────────────────────────────

  if (docs.length > 20) {
    tests.push({
      name: "Real-semantic: unfair business practices → antitrust/competition",
      category: "semantic",
      query: "company abusing dominant market position to crush competitors",
      expected_hits: [],
      min_results: 1,
      limit: 20,
    });

    tests.push({
      name: "Real-semantic: worker fired illegally → employment law",
      category: "semantic",
      query: "employee terminated in retaliation for reporting safety violations",
      expected_hits: [],
      min_results: 1,
      limit: 20,
    });

    tests.push({
      name: "Real-semantic: hiding corporate losses → SEC enforcement",
      category: "semantic",
      query: "management concealing financial losses from investors and auditors",
      expected_hits: [],
      min_results: 1,
      limit: 20,
    });
  }

  // ── Source filtering ───────────────────────────────────────────

  if (clDocs.length > 0) {
    tests.push({
      name: "Real-filter: doc_type=filing returns court opinions",
      category: "filtered",
      query: "damages liability judgment",
      filters: { doc_type: "filing" },
      expected_hits: [],
      expect_result_type: "filing",
      min_results: 1,
      limit: 20,
    });
  }

  if (edgarDocs.length > 0) {
    tests.push({
      name: "Real-filter: doc_type=document returns SEC filings",
      category: "filtered",
      query: "risk factor disclosure material",
      filters: { doc_type: "document" },
      expected_hits: [],
      expect_result_type: "document",
      min_results: 1,
      limit: 20,
    });
  }

  // ── Negative ────────────────────────────────────────────────────

  tests.push({
    name: "Real-negative: cooking recipe should return nothing relevant",
    category: "negative",
    query: "sourdough bread recipe yeast fermentation oven temperature",
    expected_hits: [],
    limit: 10,
  });

  tests.push({
    name: "Real-negative: sports scores should return nothing relevant",
    category: "negative",
    query: "football world cup semifinal goal penalty shootout",
    expected_hits: [],
    limit: 10,
  });

  return tests;
}
