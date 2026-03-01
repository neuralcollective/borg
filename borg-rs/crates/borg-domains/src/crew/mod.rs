use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, setup_phase};

pub fn crew_mode() -> PipelineMode {
    PipelineMode {
        name: "crewborg".into(),
        label: "Talent Search".into(),
        category: "People & Ops".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::None,
        default_max_attempts: 3,
        phases: vec![
            setup_phase("source"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("candidates.md".into()),
                ..agent_phase(
                    "source",
                    "Sourcing",
                    CREW_SOURCE_SYSTEM,
                    CREW_SOURCE_INSTRUCTION,
                    "Read,Glob,Grep,Write,WebSearch,WebFetch",
                    "evaluate",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "eval: candidate evaluations from crew agent".into(),
                ..agent_phase(
                    "evaluate",
                    "Evaluation",
                    CREW_EVALUATE_SYSTEM,
                    CREW_EVALUATE_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
                    "rank",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "rank: prioritized shortlist from crew agent".into(),
                ..agent_phase(
                    "rank",
                    "Ranking",
                    CREW_RANK_SYSTEM,
                    CREW_RANK_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "discovery".into(),
                label: "Candidate Discovery".into(),
                output_type: SeedOutputType::Task,
                prompt: CREW_SEED_DISCOVERY.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "refresh".into(),
                label: "Re-evaluate Pool".into(),
                output_type: SeedOutputType::Task,
                prompt: CREW_SEED_REFRESH.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "criteria".into(),
                label: "Refine Criteria".into(),
                output_type: SeedOutputType::Proposal,
                prompt: CREW_SEED_CRITERIA.into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

const CREW_SOURCE_SYSTEM: &str =
    "You are a talent sourcing agent. Your job is to find real, verifiable\
\ncandidates that match the brief. Use web search to locate profiles,\
\nportfolios, GitHub accounts, LinkedIn, personal sites, and relevant\
\ncommunities. Do not invent candidates — only record those you can verify.";

const CREW_SOURCE_INSTRUCTION: &str = "Read the task brief and search for matching candidates.\
\nWrite candidates.md containing:\
\n1. Brief (role, requirements, key criteria)\
\n2. Sourcing channels searched\
\n3. Candidate list — for each: name, profile URL(s), location, and why they match\
\nAim for 10-20 candidates. Prefer quality over quantity.";

const CREW_EVALUATE_SYSTEM: &str =
    "You are a talent evaluation agent. Read candidates.md and do deeper\
\nresearch on each candidate. Assess fit against the brief criteria.\
\nBe honest about gaps and uncertainties. Do not inflate scores.";

const CREW_EVALUATE_INSTRUCTION: &str = "Read candidates.md and evaluate each candidate.\
\nFor each, research their background, work quality, and relevance.\
\nAppend an evaluation section to candidates.md with:\
\n- Score (1-10) per key criterion\
\n- Overall fit score\
\n- Strengths and gaps\
\n- Availability signals (if findable)\
\n- Red flags if any";

const CREW_RANK_SYSTEM: &str = "You are a talent ranking agent. Synthesise the evaluations from\
\ncandidates.md into a final prioritised shortlist. Be concise and decisive.\
\nThe shortlist is the deliverable — make it actionable.";

const CREW_RANK_INSTRUCTION: &str = "Read candidates.md with evaluations and produce shortlist.md:\
\n1. Top picks (ranked 1-5) — name, score, 2-sentence summary, contact/profile link\
\n2. Reserves (next 5) — brief note on each\
\n3. Not recommended — list names and one-line reason\
\n4. Recommended next steps (outreach order, questions to ask)";

const CREW_SEED_DISCOVERY: &str = "Review the existing candidate pool in this repository.\
\nIdentify gaps: roles not yet sourced, underrepresented skill sets,\
\nor geographies not yet searched. Create a task to source candidates\
\nfor the most critical gap.";

const CREW_SEED_REFRESH: &str = "Check candidates in the existing shortlist for staleness.\
\nSearch for any whose profiles or availability may have changed\
\nsince last evaluated. Create a task to re-evaluate those candidates.";

const CREW_SEED_CRITERIA: &str =
    "Review the search briefs and evaluation criteria in this repository.\
\nSuggest 1-2 improvements: criteria that are too vague, missing signals\
\nthat would better predict fit, or sourcing channels not yet tried.";
