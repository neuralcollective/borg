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
            setup_phase("implement"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                commits: true,
                commit_message: "talent: sourcing and evaluation from crew agent".into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    CREW_IMPLEMENT_SYSTEM,
                    CREW_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
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

const CREW_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous talent sourcing agent. Source candidates, evaluate them as \
you go, and produce a ranked shortlist — all in one pass. Use web search to find \
real, verifiable candidates. Do not invent candidates. Drop poor matches early \
and stop searching once you have enough quality candidates.";

const CREW_IMPLEMENT_INSTRUCTION: &str = "\
Handle this talent search end-to-end:
1. Read the task brief (role, requirements, key criteria)
2. Search for matching candidates via web search, GitHub, LinkedIn, communities
3. Evaluate each candidate as you find them — score against criteria, note strengths/gaps
4. Drop poor matches early, keep searching if quality is low
5. Produce two files:
   - candidates.md: full list with evaluations (name, profile URLs, scores, strengths, gaps)
   - shortlist.md: top 5 ranked picks, 5 reserves, not-recommended list, recommended next steps

Aim for 10-20 candidates sourced, evaluated inline, with a clear final ranking.\n\
If the brief is unclear, write {\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json.";

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
