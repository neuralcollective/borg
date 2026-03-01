use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, setup_phase};

pub fn chef_mode() -> PipelineMode {
    PipelineMode {
        name: "chefborg".into(),
        label: "Recipe".into(),
        category: "Creative".into(),
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
                commit_message: "recipe: research and draft from chef agent".into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    CHEF_IMPLEMENT_SYSTEM,
                    CHEF_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
                    "review",
                )
            },
            PhaseConfig {
                error_instruction: CHEF_REVIEW_RETRY.into(),
                commits: true,
                commit_message: "review: revisions from chef review agent".into(),
                fresh_session: true,
                ..agent_phase(
                    "review",
                    "Review",
                    CHEF_REVIEW_SYSTEM,
                    CHEF_REVIEW_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "technique_research".into(),
                label: "Technique Research".into(),
                output_type: SeedOutputType::Task,
                prompt: CHEF_SEED_TECHNIQUE.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "seasonal".into(),
                label: "Seasonal Update".into(),
                output_type: SeedOutputType::Task,
                prompt: CHEF_SEED_SEASONAL.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "simplify".into(),
                label: "Simplify Recipe".into(),
                output_type: SeedOutputType::Task,
                prompt: CHEF_SEED_SIMPLIFY.into(),
                allowed_tools: "Read,Glob,Grep,Bash".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "elevate".into(),
                label: "Elevate Quality".into(),
                output_type: SeedOutputType::Proposal,
                prompt: CHEF_SEED_ELEVATE.into(),
                allowed_tools: "Read,Glob,Grep,Bash".into(),
                target_primary_repo: false,
            },
        ],
    }
}

const CHEF_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous culinary agent. You research techniques and ingredients \
from real sources, then create or iterate on recipes with professional precision. \
Use grams over cups, give exact temperatures and timings, and explain the why \
behind each technique. Do not invent facts — verify with web search.\n\
\n\
The task brief tells you what to optimise for. Common axes:\n\
- Quality/technique: apply professional or Michelin-level methods\n\
- Ease/speed: minimise active time, equipment, and complexity\n\
- Dietary: adapt for restrictions (vegan, gluten-free, low-sodium, etc.)\n\
- Cost: use affordable substitutes without sacrificing core flavour\n\
\n\
When iterating on an existing recipe, read it first, identify what changes serve \
the goal, and explain your reasoning in research.md.";

const CHEF_IMPLEMENT_INSTRUCTION: &str = "\
Handle this recipe task end-to-end:
1. Read the task brief and any existing recipes in the repo
2. Research relevant techniques, ingredients, or cuisines via web search
3. Write research.md: technique notes, source URLs, key findings, rationale for choices
4. Write recipe.md with this structure:
   - Title, yield, total/active time, difficulty
   - Ingredients list (grams, mL — no cups) with prep notes
   - Equipment needed
   - Method: numbered steps with temperatures (°C), timings, and visual/tactile cues
   - Notes: storage, make-ahead, variations, common mistakes
5. If iterating on an existing recipe, clearly note what changed and why

If the task is unclear, write {\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json.";

const CHEF_REVIEW_SYSTEM: &str = "\
You are a culinary editor. Read recipe.md and research.md critically. \
Check for internal consistency, technique accuracy, and whether the stated \
optimisation goal was actually met. Fix problems directly — do not just list them.";

const CHEF_REVIEW_INSTRUCTION: &str = "\
Review recipe.md against research.md. Check:\n\
1. Do ingredient quantities make sense for the stated yield?\n\
2. Are temperatures, timings, and technique descriptions accurate?\n\
3. Is the method order logical — no ingredient used before it's prepped?\n\
4. Does the recipe actually deliver on the task's optimisation goal?\n\
5. Are measurements in metric (grams, mL, °C)?\n\
6. Is the recipe self-contained — could someone cook from it without guessing?\n\
Fix any issues directly. Leave a brief review note at the top of recipe.md.";

const CHEF_REVIEW_RETRY: &str =
    "\n\nPrevious review flagged unresolved issues:\n{ERROR}\n\nAddress them.";

const CHEF_SEED_TECHNIQUE: &str = "Review the recipes in this repository.\
\nIdentify 1-3 recipes where a specific technique could be improved — \
\ne.g. better emulsification, more effective browning, improved dough hydration.\
\nCreate a task to research the technique and update the recipe.";

const CHEF_SEED_SEASONAL: &str = "Check the recipes in this repository against \
\nthe current season. Identify ingredients that are out of season or recipes \
\nthat could benefit from seasonal swaps. Create a task to update 1-2 recipes \
\nwith seasonal alternatives or propose a new seasonal recipe.";

const CHEF_SEED_SIMPLIFY: &str = "Review recipes in this repository for unnecessary complexity.\
\nIdentify 1-2 recipes where steps can be consolidated, equipment reduced, \
\nor active time cut without meaningfully affecting the result.\
\nCreate a task to simplify them.";

const CHEF_SEED_ELEVATE: &str = "Review recipes in this repository.\
\nSuggest 1-2 where a professional technique would meaningfully improve the result — \
\ne.g. sous vide for temperature control, fermentation for depth of flavour, \
\nor proper stock-making instead of store-bought. Explain the expected improvement.";
