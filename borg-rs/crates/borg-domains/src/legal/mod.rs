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
        integration: IntegrationType::None,
        default_max_attempts: 3,
        phases: vec![
            setup_phase("research"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("research.md".into()),
                ..agent_phase("research", "Research", LEGAL_RESEARCH_SYSTEM, LEGAL_RESEARCH_INSTRUCTION, "Read,Glob,Grep,Write,WebSearch,WebFetch", "draft")
            },
            PhaseConfig {
                commits: true,
                commit_message: "draft: legal document from drafting agent".into(),
                ..agent_phase("draft", "Drafting", LEGAL_DRAFT_SYSTEM, LEGAL_DRAFT_INSTRUCTION, "Read,Glob,Grep,Write,Edit", "review")
            },
            PhaseConfig {
                error_instruction: LEGAL_REVIEW_RETRY.into(),
                commits: true,
                commit_message: "review: revisions from review agent".into(),
                ..agent_phase("review", "Review", LEGAL_REVIEW_SYSTEM, LEGAL_REVIEW_INSTRUCTION, "Read,Glob,Grep,Write,Edit", "done")
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "clause_review".into(),
                label: "Clause Review".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the legal documents in this repository. Identify 1-3 specific\nclauses, provisions, or terms that could be improved, clarified, or\nthat pose legal risk. Focus on practical, actionable improvements.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "compliance".into(),
                label: "Compliance Audit".into(),
                output_type: SeedOutputType::Task,
                prompt: "Audit the documents for compliance gaps against relevant regulations,\nstandards, and best practices. Create a task for each genuine compliance\nissue found. Be specific about the regulation or standard being violated.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "precedent".into(),
                label: "Precedent Research".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Analyze the legal matters addressed in this repository. Suggest 1-3\nresearch directions for relevant case law, statutory authority, or\nregulatory guidance that could strengthen the legal positions taken.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "risk".into(),
                label: "Risk Assessment".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Perform a risk assessment of the legal documents and matters. Identify\n1-2 significant legal risks, exposure areas, or positions that could\nbe challenged. Focus on material risks, not theoretical ones.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

const LEGAL_RESEARCH_SYSTEM: &str = "You are the research agent in an autonomous legal pipeline.\nAnalyze the legal issue, research relevant law, precedent, and context,\nthen produce a research memo (research.md) at the workspace root.\nDo not draft legal documents yet — focus on thorough analysis.";

const LEGAL_RESEARCH_INSTRUCTION: &str = "Write research.md containing:\n1. Issue summary (2-3 sentences)\n2. Relevant statutes, regulations, and rules\n3. Key case law and precedent (with citations)\n4. Analysis of how the law applies to this matter\n5. Open questions and areas requiring further research";

const LEGAL_DRAFT_SYSTEM: &str = "You are the drafting agent in an autonomous legal pipeline.\nRead research.md and draft the requested legal document.\nFocus on accuracy, completeness, and proper legal formatting.\nCite sources from the research memo where applicable.";

const LEGAL_DRAFT_INSTRUCTION: &str = "Read research.md and draft the legal document described in the task.\nFollow standard legal formatting conventions.\nInclude all necessary sections, clauses, and provisions.\nCite relevant authority from the research memo.";

const LEGAL_REVIEW_SYSTEM: &str = "You are the review agent in an autonomous legal pipeline.\nReview the drafted document for legal accuracy, completeness, and quality.\nFix any issues you find directly in the document.";

const LEGAL_REVIEW_INSTRUCTION: &str = "Review all documents in the workspace for:\n1. Legal accuracy and correctness\n2. Completeness — all required sections present\n3. Internal consistency\n4. Proper citations and references\n5. Potential risks or weaknesses\n6. Formatting and style\n\nFix any issues directly. Do not just list problems — resolve them.";

const LEGAL_REVIEW_RETRY: &str = "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues in the document.";
