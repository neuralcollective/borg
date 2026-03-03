pub mod lexis;
pub mod statenet;
pub mod lexmachina;
pub mod intelligize;
pub mod cognitive;
pub mod courtlistener;
pub mod citations;
pub mod edgar;
pub mod federal_register;
pub mod westlaw;
pub mod clio;

use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, setup_phase};

fn legal_system(base: &str) -> String {
    format!("{base}{LEGAL_TOOL_INVENTORY}")
}

pub fn legal_mode() -> PipelineMode {
    PipelineMode {
        name: "lawborg".into(),
        label: "Legal".into(),
        category: "Professional Services".into(),
        initial_status: "backlog".into(),
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::GitBranch,
        default_max_attempts: 3,
        phases: vec![
            setup_phase("implement"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                commits: true,
                commit_message: "legal: research, analysis, and draft".into(),
                error_instruction: LEGAL_IMPLEMENT_RETRY.into(),
                system_prompt: legal_system(LEGAL_IMPLEMENT_SYSTEM),
                ..agent_phase(
                    "implement",
                    "Implement",
                    "",
                    LEGAL_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
                    "review",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "review: revisions from independent review".into(),
                fresh_session: true,
                error_instruction: LEGAL_REVIEW_RETRY.into(),
                system_prompt: legal_system(LEGAL_REVIEW_SYSTEM),
                ..agent_phase(
                    "review",
                    "Review",
                    "",
                    LEGAL_REVIEW_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "clause_review".into(),
                label: "Clause Review".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the legal documents in this repository. Identify 1-3 specific \
                    clauses, provisions, or terms that could be improved, clarified, or \
                    that pose legal risk. Use courtlistener_search_opinions and \
                    courtlistener_citation_lookup to check whether cited cases exist. \
                    If lexis_shepards or westlaw_keycite are available, use them for \
                    negative treatment analysis (overruled, criticized, distinguished).".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "compliance".into(),
                label: "Compliance Audit".into(),
                output_type: SeedOutputType::Task,
                prompt: "Audit the documents for compliance gaps against relevant regulations. \
                    Use federal_register_search for current federal rules, regulations_search_documents \
                    for pending regulatory actions, and uk_legislation_search or eurlex_search for \
                    international compliance. Create a task for each genuine compliance issue found.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "precedent".into(),
                label: "Precedent Research".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Analyze the legal matters in this repository. Use courtlistener_search_opinions \
                    for US case law, canlii_search for Canadian law, eurlex_search for EU law. \
                    If LexisNexis or Westlaw tools are available, use those for deeper research. \
                    Suggest 1-3 research directions that could strengthen the legal positions.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "risk".into(),
                label: "Risk Assessment".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Perform a risk assessment of the legal documents and matters. \
                    Use courtlistener_search_dockets to find similar cases and outcomes. \
                    If Lex Machina tools are available, use lexmachina_search_cases for litigation analytics. \
                    Identify 1-2 significant legal risks.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "litigation_strategy".into(),
                label: "Litigation Strategy".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Analyze the litigation aspects of this matter. Use courtlistener_search_judges \
                    for judge info and courtlistener_search_dockets for similar cases. \
                    If Lex Machina is available, use lexmachina_judge_profile for ruling patterns \
                    and lexmachina_search_cases for outcome analytics. Propose 1-2 strategic recommendations.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "regulatory_monitor".into(),
                label: "Regulatory Monitor".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Scan for recent regulatory, legislative, and case-law developments that \
                    could materially affect the legal matters in this repository. Check multiple \
                    sources: use federal_register_search for new federal rules and proposed \
                    rulemakings, congress_search_bills for pending US legislation, \
                    openstates_search_bills for relevant state-level bills, \
                    courtlistener_search_opinions for significant recent decisions in the \
                    relevant area of law, eurlex_search for EU directives or CJEU rulings, \
                    and uk_legislation_search for UK statutory instruments or amendments. \
                    Limit your search to developments from the past 90 days. \
                    Only generate a proposal if a development is both recent and genuinely \
                    impactful — meaning it directly alters an applicable legal standard, \
                    creates a new compliance obligation, or overturns a relied-upon authority. \
                    Do NOT flag developments that are speculative, tangential, or already \
                    reflected in the documents. Generate at most 3 proposals total; \
                    if nothing material is found, generate none.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "sec_compliance".into(),
                label: "SEC Compliance".into(),
                output_type: SeedOutputType::Task,
                prompt: "Audit SEC filing compliance for entities referenced in this repository. \
                    Use edgar_fulltext_search and edgar_company_filings to review filings. \
                    If Intelligize is available, use intelligize_search_clauses to compare \
                    clause language across peers. Flag any compliance concerns.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "citation_check".into(),
                label: "Citation Check".into(),
                output_type: SeedOutputType::Task,
                prompt: "Find all legal citations in the documents in this repository. \
                    Use courtlistener_citation_lookup to confirm each US citation exists in the database. \
                    If lexis_shepards is available, use it for full Shepard's treatment (good law / \
                    overruled / criticized / distinguished). If westlaw_keycite is available, use it \
                    for KeyCite status. Note: courtlistener_citation_lookup confirms existence only, \
                    not whether the case is still good law. Create a task to update any problematic citations.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "ip_review".into(),
                label: "IP Review".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review intellectual property aspects of the matters in this repository. \
                    Use uspto_search_patents and uspto_search_trademarks to check for relevant \
                    IP filings, potential conflicts, and prior art. Create a task for each \
                    IP concern that needs attention.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "conveyancing".into(),
                label: "Conveyancing".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the property transaction documents in this repository for conveyancing issues. \
                    Check title deeds, contracts of sale, and transfer documents for completeness and accuracy. \
                    Verify encumbrances, easements, covenants, and zoning compliance. \
                    Use uk_legislation_search for relevant property law statutes (Land Registration Act, \
                    Law of Property Act, etc.) or eurlex_search for cross-border property directives. \
                    Use courtlistener_search_opinions for US real property case law if applicable. \
                    Check for outstanding charges, liens, or restrictions on title. \
                    Create a task for each defect, risk, or missing item that needs resolution before completion.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

/// Returns a legal-aware system prompt suffix for chat agents.
pub fn legal_chat_system_suffix() -> &'static str {
    LEGAL_TOOL_INVENTORY
}

// ── Tool inventory appended to every legal system prompt ────────────
const LEGAL_TOOL_INVENTORY: &str = "\n\n\
## Legal Research Toolkit (MCP)\n\
\n\
FREE (always available):\n\
- courtlistener_search_opinions / courtlistener_get_opinion — US case law (federal + state)\n\
- courtlistener_citation_lookup — confirms a citation resolves to a known case (existence only, NOT treatment/good-law status)\n\
- courtlistener_search_dockets / courtlistener_get_docket — federal court dockets (RECAP)\n\
- courtlistener_search_judges / courtlistener_get_judge — judge profiles\n\
- courtlistener_search_oral_arguments — oral argument recordings\n\
- courtlistener_search_recap_documents — PACER documents in RECAP\n\
- edgar_fulltext_search / edgar_company_filings / edgar_company_facts / edgar_company_concept / edgar_resolve_ticker — SEC EDGAR\n\
- federal_register_search / federal_register_get_document / federal_register_get_agency — Federal Register\n\
- regulations_search_documents / regulations_get_document / regulations_search_dockets / regulations_get_comments — regulations.gov\n\
- congress_search_bills / congress_get_bill / congress_get_bill_text / congress_search_members — Congress.gov\n\
- uk_legislation_search / uk_legislation_get / uk_legislation_changes — UK statutes and SIs\n\
- eurlex_search / eurlex_get_document — EU legislation, directives, CJEU case law\n\
- openstates_search_bills / openstates_get_bill / openstates_search_legislators — US state legislation\n\
- canlii_search / canlii_get_case / canlii_case_citations / canlii_get_legislation — Canadian law\n\
- uspto_search_patents / uspto_get_patent / uspto_search_trademarks — US patents and trademarks\n\
\n\
PREMIUM (available when configured — use proactively if present):\n\
- lexis_shepards — Shepard's citation treatment (FULL negative treatment: good law / overruled / criticized / distinguished)\n\
- lexis_search / lexis_retrieve — LexisNexis case law and statutes\n\
- westlaw_keycite — KeyCite citation treatment (FULL negative treatment analysis)\n\
- westlaw_search / westlaw_get_document / westlaw_practical_law / westlaw_litigation_analytics — Westlaw\n\
- statenet_* — State Net legislation tracking\n\
- lexmachina_* — Lex Machina litigation analytics\n\
- intelligize_* — Intelligize SEC filings\n\
- cognitive_* — entity resolution, PII redaction, translation\n\
- clio_* — Clio practice management\n\
- imanage_* / netdocuments_* — document management\n\
- alb_* — OneAdvanced Legal (ALB) practice and case management\n\
\n\
CONNECTORS (available when configured via external MCP servers):\n\
- DocuSign — e-signature, envelope management, template sending\n\
- Box — cloud document storage, collaboration, metadata search\n\
- Slack — team messaging, channel search, legal team notifications\n\
- Microsoft 365 — Outlook email/calendar, SharePoint, OneDrive, Teams\n\
- Google Calendar — meeting schedules, availability, event management\n\
- Gmail — email search, drafts, thread management\n\
- Atlassian (Jira/Confluence) — project tracking, knowledge base\n\
- Egnyte — enterprise file sharing and governance\n\
If connector tools appear in your available tools list, use them proactively \
for document retrieval, meeting prep, communication, and workflow automation.\n\
\n\
CITATION VERIFICATION:\n\
- courtlistener_citation_lookup confirms a case EXISTS in the database. It does NOT tell you if the case is still good law.\n\
- lexis_shepards provides FULL Shepard's treatment (overruled, criticized, distinguished, followed). Use this if available.\n\
- westlaw_keycite provides FULL KeyCite treatment. Use this if available.\n\
- If neither Shepard's nor KeyCite is available, note this limitation explicitly in your output.\n\
\n\
TOOL PRIORITY:\n\
1. Premium tools FIRST (the user pays for these)\n\
2. Free MCP tools second\n\
3. WebSearch / WebFetch last — for supplementary research or gaps in MCP coverage\n\
\n\
RULES:\n\
- Every legal authority must include a source URL or database identifier.\n\
- Never rely solely on training data — verify with primary sources.\n\
- If the task is missing critical context, write {\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json BEFORE starting.";

// ── Phase system prompts ────────────────────────────────────────────
const LEGAL_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous legal research and drafting agent. You handle the full legal \
workflow: research, citation verification, analysis, and document drafting. \
Use your legal research tools extensively — never rely on training data for legal authorities. \
Every citation must be verified against a live database and include a source URL.";

const LEGAL_IMPLEMENT_INSTRUCTION: &str = "\
Handle this legal task end-to-end.

## Step 0: Assess Context

Check whether the task description specifies:
- Jurisdiction (which country, state, or court system)
- Document type (memo, brief, demand letter, contract analysis, regulatory analysis, case brief)
- Parties involved
- Specific legal questions to answer

If any critical context is missing, write {\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json \
and stop. Do not guess jurisdiction or document type.

## Step 1: Research

Search systematically. Use premium tools first if available, then free MCP tools, then web.

For each source you find, record in your notes:
- Which tool and query you used
- How many results were returned
- Which results you selected and why

Research checklist:
- Case law: courtlistener_search_opinions (US), canlii_search (Canada), eurlex_search (EU)
- Statutes: uk_legislation_search (UK), congress_search_bills (US federal), openstates_search_bills (US state)
- Regulations: federal_register_search, regulations_search_documents
- Corporate: edgar_fulltext_search, edgar_company_filings (if relevant)
- IP: uspto_search_patents, uspto_search_trademarks (if relevant)

## Step 2: Write research.md

Structure:
```
# Research Memo

**Matter:** [from task]
**Jurisdiction:** [identified jurisdiction]
**Date:** [current date]
**Confidentiality:** [PRIVILEGED AND CONFIDENTIAL — ATTORNEY WORK PRODUCT, if applicable]

## Issue Presented
[Precise statement of the legal question(s)]

## Short Answer
[Brief answer to each question — 1-2 sentences each]

## Key Authorities
[For each authority, include:]
- Full citation in Bluebook format
- Source URL or database identifier
- Verification status: Verified (tool used) / Existence confirmed (CourtListener) / Unverified
- Brief relevance note

## Discussion
[IRAC analysis: Issue → Rule → Application → Conclusion for each question]
[Tag each legal proposition with an inline confidence marker: Confidence: High / Medium / Low]

## Methodology
[Which databases were searched, what queries were used, how many results reviewed, what was excluded and why]
```

## Step 3: Verify Citations

For EVERY case, statute, and regulation cited:
- Use lexis_shepards or westlaw_keycite if available (provides full treatment analysis)
- Otherwise use courtlistener_citation_lookup (confirms existence only — note this limitation)
- Flag any case that is overruled, criticized, or has negative treatment
- Remove or replace any authority that is no longer good law
- If neither Shepard's nor KeyCite is available, add this note to research.md: \
  \"Note: Citation treatment analysis (Shepard's/KeyCite) was not available for this research. \
  Citations have been confirmed to exist via CourtListener but negative treatment has not been checked. \
  Independent verification of citation currency is recommended.\"

## Step 4: Draft the Document

For every legal claim, conclusion, or cited proposition in the document, add an inline
confidence marker immediately after the sentence or clause it applies to:
- **Confidence: High** — well-established law, verified citation (Shepard's/KeyCite), strong binding precedent
- **Confidence: Medium** — some uncertainty, developing area, conflicting authority, or existence-only verification
- **Confidence: Low** — novel theory, limited authority, training-data-only citation, or highly jurisdiction-dependent

The dashboard renders these markers as colored badges; place them naturally in prose, e.g.:
> The employer bears the burden of proving the exemption applies. *Corning Glass Works v. Brennan*, 417 U.S. 188, 196-97 (1974). Confidence: High
> Courts are divided on whether economic loss alone triggers liability. Confidence: Medium

Follow Bluebook citation format throughout:
- Cases: *Smith v. Jones*, 550 U.S. 124, 130 (2007)
- Statutes: 42 U.S.C. § 1983 (2018)
- Regulations: 17 C.F.R. § 240.10b-5 (2023)
- UK cases: [2021] UKSC 35
- EU cases: Case C-131/12, *Google Spain*, ECLI:EU:C:2014:317
- Canadian cases: *R v. Oakes*, [1986] 1 SCR 103
- Use pinpoint citations (specific page/paragraph) whenever possible
- Use *id.* and *supra* for repeated citations per Bluebook rules

If the task specifies a document type, follow its standard structure:

**Research Memo:** Issue → Short Answer → Facts → Discussion (IRAC) → Conclusion
**Case Brief:** Caption → Facts → Procedural History → Issue → Holding → Reasoning → Disposition
**Demand Letter:** Facts → Legal Basis → Specific Demand → Deadline → Consequences
**Contract Analysis:** Parties → Key Terms → Obligations → Risk Areas → Recommendations
**Motion/Brief:** Caption → Statement of Facts → Argument (with headings) → Relief Requested
**Regulatory Analysis:** Regulation → Applicability → Compliance Status → Gaps → Remediation Steps

If document type is not specified, default to Research Memo format.

### Task-Type-Specific Workflow

Read the Task Type from the task description. Adapt your workflow accordingly:

**Research Memo / Case Brief:** Standard research → draft → cite-check flow. Produce thorough IRAC analysis.

**Contract Analysis:** Focus on extraction first:
1. Identify all parties, dates, terms, and key obligations
2. Flag risk areas with severity ratings (High/Medium/Low)
3. Write specific recommendations for each risk area
4. Write `structured.json` with extracted terms:
```json
{\"task_type\":\"contract_analysis\",\"parties\":[{\"name\":\"\",\"role\":\"\"}],
\"effective_date\":\"\",\"term\":\"\",\"termination_triggers\":[\"\"],
\"key_obligations\":[{\"party\":\"\",\"obligation\":\"\"}],
\"indemnification\":{\"cap\":\"\",\"carve_outs\":[\"\"]},
\"change_of_control\":\"\",\"risk_flags\":[{\"description\":\"\",\"severity\":\"High|Medium|Low\",\"recommendation\":\"\"}]}
```

**Demand Letter:** Focus on factual precision and tone:
1. Research establishes legal basis for the demand
2. Draft follows formal demand letter structure: Facts → Legal Basis → Specific Demand → Deadline → Consequences
3. Tone is firm but professional — avoid inflammatory language
4. Include specific dollar amounts or relief requested where applicable

**Motion / Brief:** Focus on persuasive authority:
1. Research must identify the strongest binding precedent first
2. Address likely counter-arguments proactively
3. Structure follows court rules for the jurisdiction
4. Include proposed order or relief section

**Regulatory Analysis:** Focus on compliance mapping:
1. Identify all applicable regulations using federal_register_search, regulations_search_documents, eurlex_search
2. Map each regulation to the client's current compliance status
3. Flag gaps with remediation timelines
4. Track pending regulatory changes that could alter requirements
5. Write `structured.json`:
```json
{\"task_type\":\"regulatory_analysis\",\"regulations\":[{\"name\":\"\",\"citation\":\"\",\"status\":\"compliant|gap|pending\",\"notes\":\"\",\"remediation\":\"\"}],
\"pending_changes\":[{\"source\":\"\",\"description\":\"\",\"effective_date\":\"\",\"impact\":\"\"}]}
```

**Contract Review:** Playbook-based clause-by-clause analysis:
1. Identify contract type and determine which side the client is on
2. Analyze each material clause against standard positions (use the Methodology section for classification)
3. Flag deviations as GREEN/YELLOW/RED with specific redline language for YELLOW and RED
4. Generate business impact summary and negotiation strategy (lead with strongest points, identify concession candidates)
5. Write `structured.json` with contract_review output

**NDA Triage:** Rapid pre-screening for routing:
1. Quick screen against 10 dimensions (use the Methodology section)
2. Classify as GREEN (approve), YELLOW (counsel review), or RED (full legal review)
3. Generate triage report with screening results table and routing recommendation
4. Flag any non-NDA commercial terms (auto-RED)

**Compliance Review:** Privacy and regulatory compliance audit:
1. Identify all applicable regulations (GDPR, CCPA/CPRA, LGPD, POPIA, PIPEDA, etc.)
2. Review DPAs against checklist, evaluate data subject request procedures
3. Assess cross-border data transfer mechanisms (SCCs, adequacy decisions, BCRs)
4. Write compliance status report with gap analysis and remediation priorities

**Risk Assessment:** Severity-by-likelihood legal risk analysis:
1. Identify and describe each risk with background
2. Score severity (1-5) and likelihood (1-5), compute risk score
3. Classify: GREEN (1-4), YELLOW (5-9), ORANGE (10-15), RED (16-25)
4. Document mitigation options and recommended approach for each risk
5. Write `structured.json` with risk_assessment output

**Vendor Check:** Cross-system vendor agreement status:
1. Search for all agreements with the vendor (NDA, MSA, SOW, DPA, SLA, etc.)
2. Document status of each (active, expired, in negotiation, pending signature)
3. Perform gap analysis — identify missing agreements based on relationship type
4. Flag approaching expirations (within 90 days) and surviving obligations from expired agreements

**Meeting Briefing:** Contextual briefing for legal work:
1. Identify meeting type (deal review, board meeting, regulatory, general)
2. Gather context from available documents and prior research
3. Structure briefing with participants, agenda, key documents, open issues, talking points
4. Include specific action items with owners, deadlines, and priority levels

Add confidentiality header if the task involves client matters:
\"PRIVILEGED AND CONFIDENTIAL — ATTORNEY WORK PRODUCT\"

## Step 5: Write analysis.md

```
# Analysis

## Summary of Findings
[Key conclusions, 3-5 bullet points]

## Risk Assessment
[Identified risks ranked by severity: High / Medium / Low]

## Confidence Assessment
For each major conclusion, rate:
- **High confidence**: Supported by multiple verified authorities, clear law
- **Medium confidence**: Supported by authority but with caveats, unsettled area, or treatment unchecked
- **Low confidence**: Limited authority found, emerging area of law, or based on analogous reasoning

## Citation Verification Summary
- Total citations: [N]
- Verified via Shepard's/KeyCite: [N]
- Existence confirmed via CourtListener: [N]
- Unverified (training data only): [N] — LIST THESE explicitly

## Methodology
- Databases searched: [list each tool used]
- Queries run: [list key queries]
- Results reviewed: [approximate count]
- Date of research: [current date]

## Limitations
[What was NOT covered, gaps in research, databases not available]
```

## Step 6: Extract deadlines

If you identify any deadlines, filing dates, or limitation periods, write `deadlines.json`:
```json
[{\"label\":\"...\",\"due_date\":\"YYYY-MM-DD\",\"rule_basis\":\"e.g. FRCP 12(a)(1)\"}]
```
Only include dates you can determine or calculate from source material. Do not invent dates.

## Signals

If the task is unclear or you need human input: write {\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json
If the task is already completed or not actionable: write {\"status\":\"abandon\",\"reason\":\"...\"} to .borg/signal.json";

const LEGAL_IMPLEMENT_RETRY: &str =
    "\n\nPrevious attempt failed. Error:\n```\n{ERROR}\n```\nFix the issue and ensure all output files are complete.";

const LEGAL_REVIEW_SYSTEM: &str = "\
You are an independent legal review agent. You did NOT draft these documents. \
Review with fresh eyes for legal accuracy, citation integrity, and completeness. \
Fix issues directly — do not just list them.";

const LEGAL_REVIEW_INSTRUCTION: &str = "\
Review all documents in the workspace. Complete this checklist and write review_notes.md with results.

## Review Checklist

### 1. Citation Integrity
For each citation in the documents:
- [ ] Verify the citation exists using courtlistener_citation_lookup (US cases) or the appropriate tool
- [ ] If lexis_shepards or westlaw_keycite is available, check treatment status
- [ ] Confirm Bluebook format is correct:
  - Cases: *Party v. Party*, [volume] [reporter] [page], [pinpoint] ([court] [year])
  - Statutes: [title] [code] § [section] ([year])
  - Regulations: [title] C.F.R. § [section] ([year])
- [ ] Verify every citation has a source URL or database identifier
- [ ] Flag and replace any case that is overruled or has significant negative treatment

### 2. Legal Accuracy
- [ ] Are the legal rules stated correctly for the specified jurisdiction?
- [ ] Does the analysis follow from the cited authorities?
- [ ] Are there obvious counter-arguments or adverse authority not addressed?
- [ ] Use WebSearch to check for very recent developments (last 6 months)

### 3. Completeness
- [ ] All required sections present for the document type
- [ ] Issue(s) fully answered — no dangling questions
- [ ] Remedies or recommendations are specific and actionable

### 4. Consistency
- [ ] research.md findings match the draft document's citations
- [ ] analysis.md conclusions align with the draft
- [ ] No contradictions between documents

### 5. Regulatory Currency
- [ ] Use federal_register_search to confirm cited regulations are current
- [ ] Use congress_get_bill to check if cited statutes have pending amendments
- [ ] Note any upcoming regulatory changes that could affect the analysis

### 6. Confidence Assessment Review
- [ ] Verify the confidence ratings in analysis.md are justified
- [ ] Any \"High confidence\" claim must have verified citations
- [ ] Any conclusion based on unverified citations should be Medium or Low
- [ ] Confirm inline \"Confidence: High/Medium/Low\" markers are present throughout the drafted document and research.md Discussion section
- [ ] Downgrade any \"Confidence: High\" marker on a claim whose citation could not be verified via Shepard's/KeyCite
- [ ] Add missing confidence markers to any uncovered legal proposition

## Output: review_notes.md

Write review_notes.md with:
```
# Independent Review Notes

**Reviewer:** Automated Review Agent
**Date:** [current date]

## Checklist Results
[Pass/Fail for each item above with brief notes]

## Issues Found and Fixed
[List each issue and what was changed]

## Issues Found — Requires Human Review
[Any issues the reviewer could not resolve autonomously]

## Overall Assessment
[Pass / Pass with caveats / Fail — with reasoning]
```

Fix all issues you can directly in the source documents. For issues requiring human judgment, \
document them in review_notes.md under \"Requires Human Review\".";

const LEGAL_REVIEW_RETRY: &str = "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues in the documents and update review_notes.md.";
