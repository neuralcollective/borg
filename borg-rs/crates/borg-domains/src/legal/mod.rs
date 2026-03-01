pub mod lexis;
pub mod statenet;
pub mod lexmachina;
pub mod intelligize;
pub mod cognitive;
pub mod courtlistener;
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
        uses_git_worktrees: true,
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::GitBranch,
        default_max_attempts: 3,
        phases: vec![
            setup_phase("research"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("research.md".into()),
                system_prompt: legal_system(LEGAL_RESEARCH_SYSTEM),
                ..agent_phase(
                    "research",
                    "Research",
                    "",
                    LEGAL_RESEARCH_INSTRUCTION,
                    "Read,Glob,Grep,Write,WebSearch,WebFetch",
                    "analyze",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "analyze: legal analysis with verified citations".into(),
                check_artifact: Some("analysis.md".into()),
                system_prompt: legal_system(LEGAL_ANALYZE_SYSTEM),
                ..agent_phase(
                    "analyze",
                    "Analysis",
                    "",
                    LEGAL_ANALYZE_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
                    "draft",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "draft: legal document from drafting agent".into(),
                system_prompt: legal_system(LEGAL_DRAFT_SYSTEM),
                ..agent_phase(
                    "draft",
                    "Drafting",
                    "",
                    LEGAL_DRAFT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "review",
                )
            },
            PhaseConfig {
                error_instruction: LEGAL_REVIEW_RETRY.into(),
                commits: true,
                commit_message: "review: revisions from review agent".into(),
                fresh_session: true,
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
                    courtlistener_citation_lookup to verify cited precedent is still good law. \
                    If LexisNexis tools are available, also use lexis_shepards for Shepard's treatment.".into(),
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
                output_type: SeedOutputType::Task,
                prompt: "Check for recent legislative or regulatory changes that may affect matters \
                    in this repository. Use federal_register_search for federal rules, \
                    congress_search_bills for pending legislation, openstates_search_bills for \
                    state-level changes. Create a task for each change that requires action.".into(),
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
                    Use courtlistener_citation_lookup to verify each US citation. \
                    If LexisNexis is available, use lexis_shepards for Shepard's treatment. \
                    If Westlaw is available, use westlaw_keycite for KeyCite verification. \
                    Create a task to update any citations that are no longer good law.".into(),
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
        ],
    }
}

// ── Tool inventory appended to every legal system prompt ────────────
const LEGAL_TOOL_INVENTORY: &str = "\n\n\
You have access to a comprehensive legal research toolkit via MCP:\n\
\n\
FREE (always available):\n\
- courtlistener_search_opinions / courtlistener_get_opinion / courtlistener_citation_lookup — US case law (federal + state)\n\
- courtlistener_search_dockets / courtlistener_get_docket — federal court dockets (RECAP archive)\n\
- courtlistener_search_judges / courtlistener_get_judge — judge profiles, appointments, courts\n\
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
- lexis_search / lexis_retrieve / lexis_shepards — LexisNexis case law, Shepard's citations\n\
- statenet_* — State Net legislation tracking\n\
- lexmachina_* — Lex Machina litigation analytics\n\
- intelligize_* — Intelligize SEC filings\n\
- cognitive_* — entity resolution, PII redaction, translation\n\
- westlaw_search / westlaw_get_document / westlaw_keycite / westlaw_practical_law / westlaw_litigation_analytics — Westlaw\n\
- clio_* — Clio practice management\n\
- imanage_* / netdocuments_* — document management\n\
\n\
Use these tools proactively. Do not rely solely on training data — verify with primary sources.";

// ── Phase system prompts ────────────────────────────────────────────
const LEGAL_RESEARCH_SYSTEM: &str = "\
You are the research agent in an autonomous legal pipeline.\n\
Analyze the legal issue, research relevant law, precedent, and context,\n\
then produce a research memo (research.md) at the workspace root.\n\
Do not draft legal documents yet — focus on thorough analysis.";

const LEGAL_RESEARCH_INSTRUCTION: &str = "\
Write research.md containing:\n\
1. Issue summary (2-3 sentences)\n\
2. Relevant statutes and regulations — use CourtListener, Federal Register, Congress.gov, regulations.gov\n\
3. Key case law with citations — use courtlistener_search_opinions, verify with courtlistener_citation_lookup\n\
4. If LexisNexis available, use lexis_search for deeper research and lexis_shepards for Shepard's treatment\n\
5. If Westlaw available, cross-reference with westlaw_search and westlaw_keycite\n\
6. For UK matters use uk_legislation_search, for EU use eurlex_search, for Canadian use canlii_search\n\
7. Regulatory landscape — federal_register_search, openstates_search_bills for state-level\n\
8. Analysis of how the law applies\n\
9. Open questions requiring further research";

const LEGAL_ANALYZE_SYSTEM: &str = "\
You are the analysis agent in an autonomous legal pipeline.\n\
Read research.md and perform deep verification and analysis.\n\
Validate all citations, assess litigation risk, and produce analysis.md.";

const LEGAL_ANALYZE_INSTRUCTION: &str = "\
Read research.md and produce analysis.md containing:\n\
1. Citation verification — use courtlistener_citation_lookup for every cited case.\n\
   If LexisNexis available use lexis_shepards. If Westlaw available use westlaw_keycite.\n\
   Flag any overruled, distinguished, or criticized cases.\n\
2. Litigation analytics — use courtlistener_search_dockets for similar cases.\n\
   If Lex Machina available, use lexmachina_search_cases / lexmachina_judge_profile.\n\
3. Regulatory status — use federal_register_search, congress_get_bill to verify current law.\n\
   For UK use uk_legislation_changes, for EU use eurlex_get_document.\n\
4. SEC check — use edgar_fulltext_search if corporate matters. If Intelligize available, use it.\n\
5. IP check — use uspto_search_patents / uspto_search_trademarks if relevant.\n\
6. Consolidated findings and risk assessment\n\
7. Recommended approach for drafting";

const LEGAL_DRAFT_SYSTEM: &str = "\
You are the drafting agent in an autonomous legal pipeline.\n\
Read research.md and analysis.md, then draft the requested legal document.\n\
Focus on accuracy, completeness, and proper legal formatting.\n\
Only cite cases that passed verification in analysis.md.";

const LEGAL_DRAFT_INSTRUCTION: &str = "\
Read research.md and analysis.md, then draft the legal document described in the task.\n\
Follow standard legal formatting conventions.\n\
Include all necessary sections, clauses, and provisions.\n\
Only cite cases confirmed as good law in analysis.md.\n\
Use cognitive_redact_pii if available and the document contains sensitive personal information.";

const LEGAL_REVIEW_SYSTEM: &str = "\
You are the review agent in an autonomous legal pipeline.\n\
Review the drafted document for legal accuracy, completeness, and quality.\n\
Fix any issues you find directly in the document.";

const LEGAL_REVIEW_INSTRUCTION: &str = "\
Review all documents in the workspace for:\n\
1. Legal accuracy — use courtlistener_citation_lookup to re-verify key citations.\n\
   If LexisNexis available, re-check with lexis_shepards. If Westlaw available, use westlaw_keycite.\n\
2. Completeness — all required sections present\n\
3. Internal consistency between research, analysis, and the draft\n\
4. Proper citations — correct format, pinpoint cites\n\
5. Regulatory currency — use federal_register_search and congress_get_bill to confirm cited laws are current\n\
6. Potential risks or weaknesses\n\
7. Formatting and style\n\
\n\
Fix any issues directly. Do not just list problems — resolve them.";

const LEGAL_REVIEW_RETRY: &str = "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues in the document.";
