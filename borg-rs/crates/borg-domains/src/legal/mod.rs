pub mod lexis;
pub mod statenet;
pub mod lexmachina;
pub mod intelligize;
pub mod cognitive;

use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, setup_phase};

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
                ..agent_phase(
                    "research",
                    "Research",
                    LEGAL_RESEARCH_SYSTEM,
                    LEGAL_RESEARCH_INSTRUCTION,
                    "Read,Glob,Grep,Write,WebSearch,WebFetch",
                    "analyze",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "analyze: legal analysis with verified citations".into(),
                check_artifact: Some("analysis.md".into()),
                ..agent_phase(
                    "analyze",
                    "Analysis",
                    LEGAL_ANALYZE_SYSTEM,
                    LEGAL_ANALYZE_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
                    "draft",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "draft: legal document from drafting agent".into(),
                ..agent_phase(
                    "draft",
                    "Drafting",
                    LEGAL_DRAFT_SYSTEM,
                    LEGAL_DRAFT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "review",
                )
            },
            PhaseConfig {
                error_instruction: LEGAL_REVIEW_RETRY.into(),
                commits: true,
                commit_message: "review: revisions from review agent".into(),
                ..agent_phase(
                    "review",
                    "Review",
                    LEGAL_REVIEW_SYSTEM,
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
                prompt: "Review the legal documents in this repository. Identify 1-3 specific\nclauses, provisions, or terms that could be improved, clarified, or\nthat pose legal risk. Focus on practical, actionable improvements.\nUse LexisNexis tools to verify any cited precedent is still good law.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "compliance".into(),
                label: "Compliance Audit".into(),
                output_type: SeedOutputType::Task,
                prompt: "Audit the documents for compliance gaps against relevant regulations,\nstandards, and best practices. Use LexisNexis to search for current\nregulatory requirements and recent enforcement actions. Create a task\nfor each genuine compliance issue found.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "precedent".into(),
                label: "Precedent Research".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Analyze the legal matters in this repository. Use LexisNexis to search\nfor relevant case law, statutory authority, or regulatory guidance.\nSuggest 1-3 research directions that could strengthen the legal positions.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "risk".into(),
                label: "Risk Assessment".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Perform a risk assessment of the legal documents and matters.\nUse Lex Machina litigation analytics to assess exposure and outcomes\nfor similar cases. Identify 1-2 significant legal risks.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "litigation_strategy".into(),
                label: "Litigation Strategy".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Analyze the litigation aspects of this matter. Use Lex Machina to\nresearch judge ruling patterns, typical damages, and case outcomes\nfor similar disputes. Propose 1-2 strategic recommendations.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "regulatory_monitor".into(),
                label: "Regulatory Monitor".into(),
                output_type: SeedOutputType::Task,
                prompt: "Use State Net to check for recent legislative or regulatory changes\nthat may affect the matters in this repository. Create a task for\neach change that requires action or updated analysis.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "compliance_audit".into(),
                label: "SEC Compliance".into(),
                output_type: SeedOutputType::Task,
                prompt: "Use Intelligize to audit SEC filing compliance for entities referenced\nin this repository. Check for disclosure gaps, compare clause language\nacross peers, and flag any compliance concerns.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "citation_check".into(),
                label: "Citation Check".into(),
                output_type: SeedOutputType::Task,
                prompt: "Find all legal citations in the documents in this repository.\nUse Shepard's to validate each citation — check whether cases have been\noverruled, distinguished, or superseded. Create a task to update any\ncitations that are no longer good law.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

// ── System prompts ───────────────────────────────────────────────────
// Every system prompt includes the LexisNexis tool inventory so agents
// always know they have access and use it proactively.

const LEGAL_RESEARCH_SYSTEM: &str = "You are the research agent in an autonomous legal pipeline.\n\
Analyze the legal issue, research relevant law, precedent, and context,\n\
then produce a research memo (research.md) at the workspace root.\n\
Do not draft legal documents yet — focus on thorough analysis.\n\
\n\
You have LexisNexis tools available — always use them proactively:\n\
- lexis_search / lexis_retrieve / lexis_shepards — case law, secondary sources, Shepard's citations\n\
- statenet_search_bills / statenet_get_bill / statenet_search_regulations / statenet_get_statute — legislation\n\
- lexmachina_search_cases / lexmachina_case_details / lexmachina_judge_profile / lexmachina_party_history — litigation analytics\n\
- intelligize_search_filings / intelligize_get_filing / intelligize_search_clauses — SEC filings\n\
- cognitive_resolve_judge / cognitive_resolve_court / cognitive_legal_define / cognitive_redact_pii — entity resolution, PII redaction\n\
Do not rely solely on training data for legal authority — verify with LexisNexis.";

const LEGAL_RESEARCH_INSTRUCTION: &str = "Write research.md containing:\n\
1. Issue summary (2-3 sentences)\n\
2. Relevant statutes, regulations, and rules — search LexisNexis and State Net\n\
3. Key case law and precedent (with citations) — use lexis_search, verify with Shepard's\n\
4. Litigation context — use Lex Machina for relevant case analytics if applicable\n\
5. Regulatory landscape — use State Net for pending legislation that may affect the matter\n\
6. Analysis of how the law applies to this matter\n\
7. Open questions and areas requiring further research";

const LEGAL_ANALYZE_SYSTEM: &str = "You are the analysis agent in an autonomous legal pipeline.\n\
Read research.md and perform deep verification and analysis.\n\
Validate all citations, assess litigation risk using analytics,\n\
and produce analysis.md with verified findings.\n\
\n\
You have LexisNexis tools available — always use them proactively:\n\
- lexis_shepards — verify every citation is still good law\n\
- lexmachina_search_cases / lexmachina_judge_profile — litigation analytics and judge patterns\n\
- statenet_search_regulations / statenet_get_statute — verify regulatory status\n\
- intelligize_search_filings — check SEC compliance if relevant\n\
- cognitive_resolve_judge / cognitive_resolve_court — resolve ambiguous entities";

const LEGAL_ANALYZE_INSTRUCTION: &str = "Read research.md and produce analysis.md containing:\n\
1. Citation verification — use Shepard's to check every cited case is still good law.\n\
   Flag any cases that have been overruled, distinguished, or criticized.\n\
2. Litigation analytics — use Lex Machina to assess:\n\
   - Relevant judge ruling patterns (if court/judge is known)\n\
   - Typical outcomes and damages for similar cases\n\
   - Party litigation history if relevant\n\
3. Regulatory status — use State Net to verify current status of cited statutes/regulations\n\
4. Compliance check — use Intelligize if SEC filings or corporate disclosures are relevant\n\
5. Entity verification — use cognitive tools to resolve any ambiguous judge/court references\n\
6. Consolidated findings and risk assessment\n\
7. Recommended approach for the drafting phase";

const LEGAL_DRAFT_SYSTEM: &str = "You are the drafting agent in an autonomous legal pipeline.\n\
Read research.md and analysis.md, then draft the requested legal document.\n\
Focus on accuracy, completeness, and proper legal formatting.\n\
Cite sources from the research and analysis memos.\n\
Only cite cases that passed Shepard's verification in analysis.md.\n\
\n\
You have LexisNexis tools available if you need to look anything up:\n\
- lexis_retrieve — get full text of a document\n\
- cognitive_redact_pii — redact sensitive personal information\n\
- cognitive_legal_define — look up legal terms";

const LEGAL_DRAFT_INSTRUCTION: &str = "Read research.md and analysis.md, then draft the legal document described in the task.\n\
Follow standard legal formatting conventions.\n\
Include all necessary sections, clauses, and provisions.\n\
Only cite cases confirmed as good law in analysis.md.\n\
Use cognitive_redact_pii if the document contains sensitive personal information that should be redacted.";

const LEGAL_REVIEW_SYSTEM: &str = "You are the review agent in an autonomous legal pipeline.\n\
Review the drafted document for legal accuracy, completeness, and quality.\n\
Fix any issues you find directly in the document.\n\
\n\
You have LexisNexis tools available — use them to verify:\n\
- lexis_shepards — re-check key citations\n\
- statenet_get_statute — confirm statutes are current\n\
- lexmachina_search_cases — verify litigation claims\n\
- cognitive_resolve_judge / cognitive_resolve_court — verify entity references";

const LEGAL_REVIEW_INSTRUCTION: &str = "Review all documents in the workspace for:\n\
1. Legal accuracy — verify key citations one more time with Shepard's\n\
2. Completeness — all required sections present\n\
3. Internal consistency between research, analysis, and the draft\n\
4. Proper citations and references — correct format, pinpoint cites\n\
5. Regulatory currency — confirm cited statutes/regulations are still current via State Net\n\
6. Potential risks or weaknesses\n\
7. Formatting and style\n\
\n\
Fix any issues directly. Do not just list problems — resolve them.";

const LEGAL_REVIEW_RETRY: &str = "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues in the document.";
