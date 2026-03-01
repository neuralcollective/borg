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
            setup_phase("research"),
            PhaseConfig {
                include_task_context: true,
                check_artifact: Some("prospect.md".into()),
                ..agent_phase(
                    "research",
                    "Prospect Research",
                    SALES_RESEARCH_SYSTEM,
                    SALES_RESEARCH_INSTRUCTION,
                    "Read,Glob,Grep,Write,WebSearch,WebFetch",
                    "draft",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "draft: outreach from sales agent".into(),
                ..agent_phase(
                    "draft",
                    "Outreach Draft",
                    SALES_DRAFT_SYSTEM,
                    SALES_DRAFT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "review",
                )
            },
            PhaseConfig {
                error_instruction: SALES_REVIEW_RETRY.into(),
                commits: true,
                commit_message: "review: revisions from sales review agent".into(),
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

const SALES_RESEARCH_SYSTEM: &str =
    "You are a sales research agent. Research the prospect thoroughly\
\nbefore any outreach is drafted. Find recent news, product focus, team\
\nsize, funding, pain points, and relevant context. Do not fabricate facts.";

const SALES_RESEARCH_INSTRUCTION: &str =
    "Research the prospect described in the task and write prospect.md:\
\n1. Company/person overview (what they do, size, stage)\
\n2. Recent news and signals (funding, launches, hires, press)\
\n3. Likely pain points relevant to our offering\
\n4. Key decision-makers and their roles\
\n5. Recommended angle for outreach (what to lead with and why)";

const SALES_DRAFT_SYSTEM: &str = "You are a sales outreach agent. Read prospect.md and draft\
\npersonalised, concise outreach. Lead with insight, not a pitch.\
\nAvoid generic templates — every word should be specific to this prospect.";

const SALES_DRAFT_INSTRUCTION: &str = "Read prospect.md and draft outreach.md containing:\
\n1. Email subject line (sharp, specific, not clickbait)\
\n2. Email body (3-5 short paragraphs max)\
\n   - Hook: something specific and relevant to them\
\n   - Bridge: connect their situation to what we offer\
\n   - Ask: one clear, low-friction call to action\
\n3. LinkedIn message variant (under 300 chars)\
\n4. Notes on timing or personalisation to add before sending";

const SALES_REVIEW_SYSTEM: &str =
    "You are a senior sales reviewer. Read prospect.md and outreach.md.\
\nAssess the outreach for relevance, tone, personalisation, and clarity.\
\nFix weak spots directly in outreach.md. Do not just list issues.";

const SALES_REVIEW_INSTRUCTION: &str = "Review outreach.md against prospect.md. Check:\
\n1. Does the hook reference something genuinely specific to the prospect?\
\n2. Is the value prop clear and relevant to their likely pain points?\
\n3. Is the ask concrete and easy to say yes to?\
\n4. Tone: confident but not pushy, peer-to-peer\
\n5. Length: email under 200 words, LinkedIn under 300 chars\
\nFix any issues directly. Leave a brief review note at the top of outreach.md.";

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
