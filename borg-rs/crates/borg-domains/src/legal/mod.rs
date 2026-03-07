pub mod citations;
pub mod clio;
pub mod cognitive;
pub mod courtlistener;
pub mod edgar;
pub mod federal_register;
pub mod intelligize;
pub mod lexis;
pub mod lexmachina;
pub mod statenet;
pub mod westlaw;

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
                disallowed_tools: "Bash".into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    "",
                    LEGAL_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,web_search,WebFetch",
                    "review",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "review: revisions from independent review".into(),
                fresh_session: true,
                error_instruction: LEGAL_REVIEW_RETRY.into(),
                system_prompt: legal_system(LEGAL_REVIEW_SYSTEM),
                disallowed_tools: "Bash,web_search,WebFetch".into(),
                ..agent_phase(
                    "review",
                    "Review",
                    "",
                    LEGAL_REVIEW_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "human_review",
                )
            },
            PhaseConfig {
                name: "human_review".into(),
                label: "Human Review".into(),
                phase_type: borg_core::types::PhaseType::HumanReview,
                instruction: LEGAL_HUMAN_REVIEW_INSTRUCTION.into(),
                revision_target: "implement".into(),
                next: "purge".into(),
                ..Default::default()
            },
            PhaseConfig {
                name: "purge".into(),
                label: "Burn After Reading (7d)".into(),
                phase_type: borg_core::types::PhaseType::Purge,
                wait_s: Some(604800),
                next: "purged".into(),
                ..Default::default()
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
Your available MCP tools include legal research databases. Key categories:\n\
- **CourtListener** — US case law, dockets, judges, oral arguments. courtlistener_citation_lookup confirms existence only (NOT treatment).\n\
- **EDGAR** — SEC filings, company facts\n\
- **Federal Register / regulations.gov** — federal rules and proposed rulemakings\n\
- **Congress.gov / OpenStates** — federal and state legislation\n\
- **UK Legislation / EUR-Lex / CanLII** — international law\n\
- **USPTO** — patents and trademarks\n\
\n\
PREMIUM (use proactively if present — the user pays for these):\n\
- **lexis_shepards** — FULL Shepard's citation treatment (good law / overruled / criticized / distinguished)\n\
- **westlaw_keycite** — FULL KeyCite treatment\n\
- **lexis_search / westlaw_search** — comprehensive case law and statutes\n\
- **lexmachina_*** — litigation analytics\n\
- **intelligize_*** — SEC filings comparison\n\
- **kldiscovery_*** — eDiscovery, legal holds, document review\n\
- **clio_*** / **imanage_*** / **netdocuments_*** — practice/document management\n\
\n\
CITATION VERIFICATION:\n\
- courtlistener_citation_lookup = existence only. Does NOT check if still good law.\n\
- lexis_shepards or westlaw_keycite = full treatment. Use if available.\n\
- If neither is available, note this limitation in your output.\n\
\n\
TOOL PRIORITY: Premium first → Free MCP second → WebSearch last.\n\
\n\
RULES:\n\
- Every legal authority must include a source URL or database identifier.\n\
- Never rely solely on training data — verify with primary sources.\n\
- If critical context is missing, signal blocked BEFORE starting.";

// ── Phase system prompts ────────────────────────────────────────────
const LEGAL_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous legal research and drafting agent. You handle the full legal \
workflow: research, citation verification, analysis, and document drafting. \
Use your legal research tools extensively — never rely on training data for legal authorities. \
Every citation must be verified against a live database and include a source URL.";

const LEGAL_IMPLEMENT_INSTRUCTION: &str = "\
Handle this legal task end-to-end.

## Step 0: Assess Context

Check for: jurisdiction, document type, parties, specific legal questions.
If any critical context is missing, write {\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json \
and stop. Do not guess jurisdiction or document type.

## Step 1: Research

Search systematically using the Legal Research Toolkit. Follow the tool priority order \
(premium first, free MCP second, web last). Record which tools and queries you used.

## Step 2: Write research.md

Include: matter details, jurisdiction, date, issue presented, short answer, key authorities \
(full Bluebook citation + source URL + verification status for each), IRAC discussion with \
inline confidence markers (High/Medium/Low), and methodology (tools searched, queries, results count).

Add confidentiality header if the task involves client matters.

## Step 3: Verify Citations

Verify all citations using the Citation Verification workflow from the toolkit. \
Remove or replace any authority with negative treatment.

## Step 4: Draft the Document

Add inline confidence markers after each legal claim:
- **Confidence: High** — verified citation, binding precedent, well-established law
- **Confidence: Medium** — some uncertainty, existence-only verification, developing area
- **Confidence: Low** — limited authority, novel theory, training-data-only citation

Follow Bluebook citation format. Use pinpoint citations. Use *id.* and *supra* for repeats.

Document type structures:
- **Research Memo:** Issue → Short Answer → Facts → Discussion (IRAC) → Conclusion
- **Case Brief:** Caption → Facts → Procedural History → Issue → Holding → Reasoning → Disposition
- **Demand Letter:** Facts → Legal Basis → Specific Demand → Deadline → Consequences
- **Contract Analysis:** Parties → Key Terms → Obligations → Risk Areas → Recommendations
- **Motion/Brief:** Caption → Statement of Facts → Argument → Relief Requested
- **Regulatory Analysis:** Regulation → Applicability → Compliance Status → Gaps → Remediation

Default to Research Memo if unspecified. If a Methodology section was provided above, follow it.

## Step 5: Write analysis.md

Include: summary of findings (3-5 bullets), risk assessment (High/Medium/Low), \
confidence assessment for each conclusion, citation verification summary \
(verified/existence-only/unverified counts — list unverified explicitly), \
methodology (tools, queries, date), and limitations.

## Step 6: Extract deadlines

If you find deadlines or limitation periods, write `deadlines.json`:
```json
[{\"label\":\"...\",\"due_date\":\"YYYY-MM-DD\",\"rule_basis\":\"e.g. FRCP 12(a)(1)\"}]
```

## Signals

If blocked: write {\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json
If not actionable: write {\"status\":\"abandon\",\"reason\":\"...\"} to .borg/signal.json";

const LEGAL_IMPLEMENT_RETRY: &str =
    "\n\nPrevious attempt failed. Error:\n```\n{ERROR}\n```\nFix the issue and ensure all output files are complete.";

const LEGAL_REVIEW_SYSTEM: &str = "\
You are an independent legal review agent. You did NOT draft these documents. \
Review with fresh eyes for legal accuracy, citation integrity, and completeness. \
Fix issues directly — do not just list them.";

const LEGAL_REVIEW_INSTRUCTION: &str = "\
Review all documents in the workspace. Fix issues directly. Write review_notes.md with results.

## Review Checklist

1. **Citation Integrity** — Verify each citation exists and check treatment (use the Citation Verification \
workflow from the toolkit). Confirm Bluebook format. Replace any authority with negative treatment.
2. **Legal Accuracy** — Rules correct for jurisdiction? Analysis follows from authorities? Counter-arguments addressed?
3. **Completeness** — All required sections present. Issues fully answered. Recommendations actionable.
4. **Consistency** — research.md, analysis.md, and draft all agree. No contradictions.
5. **Regulatory Currency** — Confirm cited regulations are current. Note pending changes.
6. **Confidence Markers** — Verify ratings are justified. High requires verified citations. \
Add missing markers. Downgrade any High on unverified citations.

## Output: review_notes.md

Include: checklist results (pass/fail with notes), issues found and fixed, \
issues requiring human review, overall assessment (Pass / Pass with caveats / Fail).";

const LEGAL_REVIEW_RETRY: &str = "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues in the documents and update review_notes.md.";

const LEGAL_HUMAN_REVIEW_INSTRUCTION: &str = "\
Human review gate.

The agent draft and independent review are complete. A human reviewer must now:
- approve to release the draft to the retention/purge window,
- request revision to send the matter back through the full implement -> review loop, or
- reject to stop the task.

For legal work, request revision whenever the draft needs additional authority checking, factual correction, or substantive reframing. \
Revision requests are intended to be repeatable; each request should trigger another full drafting-and-review cycle before this gate is passed.";
