use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, setup_phase};

pub fn sales_mode() -> PipelineMode {
    PipelineMode {
        name: "salesborg".into(),
        label: "Sales Outreach".into(),
        category: "Professional Services".into(),
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
                commits: true,
                commit_message: "draft: outreach from sales agent".into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    SALES_IMPLEMENT_SYSTEM,
                    SALES_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
                    "review",
                )
            },
            PhaseConfig {
                error_instruction: SALES_REVIEW_RETRY.into(),
                commits: true,
                commit_message: "review: revisions from sales review agent".into(),
                fresh_session: true,
                ..agent_phase(
                    "review",
                    "Review",
                    SALES_REVIEW_SYSTEM,
                    SALES_REVIEW_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "lead_discovery".into(),
                label: "Lead Discovery".into(),
                output_type: SeedOutputType::Task,
                prompt: SALES_SEED_DISCOVERY.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "follow_up".into(),
                label: "Follow-up Drafts".into(),
                output_type: SeedOutputType::Task,
                prompt: SALES_SEED_FOLLOWUP.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "icp".into(),
                label: "ICP Refinement".into(),
                output_type: SeedOutputType::Proposal,
                prompt: SALES_SEED_ICP.into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

const SALES_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous sales agent. Research the prospect thoroughly, then draft \
personalised outreach — all in one pass. Lead with insight, not a pitch. \
Do not fabricate facts.";

const SALES_IMPLEMENT_INSTRUCTION: &str = "\
Handle this sales task end-to-end:
1. Research the prospect: company overview, recent news, funding, team, pain points
2. Identify the best angle for outreach based on your research
3. Write prospect.md with your research findings
4. Draft outreach.md containing:
   - Email subject line (sharp, specific, not clickbait)
   - Email body (3-5 short paragraphs max): hook, bridge, ask
   - LinkedIn message variant (under 300 chars)
   - Notes on timing or personalisation to add before sending\n\
If the task is unclear, write {\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json.";

const SALES_REVIEW_SYSTEM: &str = "\
You are a senior sales reviewer. Read prospect.md and outreach.md. \
Assess the outreach for relevance, tone, personalisation, and clarity. \
Fix weak spots directly in outreach.md. Do not just list issues.";

const SALES_REVIEW_INSTRUCTION: &str = "\
Review outreach.md against prospect.md. Check:\n\
1. Does the hook reference something genuinely specific to the prospect?\n\
2. Is the value prop clear and relevant to their likely pain points?\n\
3. Is the ask concrete and easy to say yes to?\n\
4. Tone: confident but not pushy, peer-to-peer\n\
5. Length: email under 200 words, LinkedIn under 300 chars\n\
Fix any issues directly. Leave a brief review note at the top of outreach.md.";

const SALES_REVIEW_RETRY: &str =
    "\n\nPrevious review flagged unresolved issues:\n{ERROR}\n\nAddress them.";

const SALES_SEED_DISCOVERY: &str = "Review the prospect list in this repository.\
\nIdentify 1-3 new leads that fit the ideal customer profile based on\
\nwhat's already here. Each lead should be a real, findable company or person.\
\nCreate a task for each with enough context to kick off research.";

const SALES_SEED_FOLLOWUP: &str = "Review prospects where outreach was sent but no reply recorded.\
\nDraft a follow-up task for the highest-priority ones.\
\nFollow-ups should add new value (a resource, insight, or hook)\
\nrather than just bumping the thread.";

const SALES_SEED_ICP: &str = "Analyse the prospect list and outreach results in this repository.\
\nSuggest 1-2 refinements to the ideal customer profile: segments that\
\nare converting better, new verticals worth targeting, or personas\
\nbeing missed. Base suggestions on what's already in the data.";
