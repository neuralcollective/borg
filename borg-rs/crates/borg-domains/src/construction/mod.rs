use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, setup_phase};

pub fn construction_mode() -> PipelineMode {
    PipelineMode {
        name: "buildborg".into(),
        label: "Construction".into(),
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
                commit_message: "build: research and analysis from buildborg agent".into(),
                error_instruction: BUILD_IMPLEMENT_RETRY.into(),
                system_prompt: BUILD_IMPLEMENT_SYSTEM.into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    "",
                    BUILD_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,web_search,WebFetch",
                    "review",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "review: revisions from build review agent".into(),
                fresh_session: true,
                error_instruction: BUILD_REVIEW_RETRY.into(),
                system_prompt: BUILD_REVIEW_SYSTEM.into(),
                ..agent_phase(
                    "review",
                    "Review",
                    "",
                    BUILD_REVIEW_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,web_search,WebFetch",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "permit_research".into(),
                label: "Permit Research".into(),
                output_type: SeedOutputType::Task,
                prompt: "Research building permit requirements for the project described in this \
                    repository. Identify the jurisdiction and use shovels_get_geography + \
                    shovels_search_permits to find similar permits in the area. Use web_search for \
                    the jurisdiction's building department website, required forms, fee schedules, \
                    and submission procedures. Check zoning compatibility using the parcel address \
                    via shovels_search_addresses. Determine: required permit types (building, \
                    electrical, plumbing, mechanical, demolition), applicable building codes \
                    (IBC/IRC edition adopted by jurisdiction), special requirements (historic \
                    district, flood zone, HOA), estimated timeline and fees, and required \
                    inspections. Write research.md with jurisdiction-specific requirements and \
                    create a task for each permit application that needs to be filed.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "contractor_search".into(),
                label: "Contractor Search".into(),
                output_type: SeedOutputType::Task,
                prompt: "Find qualified contractors for the project described in this repository. \
                    Use shovels_get_geography to identify the geo_id, then shovels_search_contractors \
                    filtered by relevant permit_tags (e.g. hvac, solar, reroof, new_dwelling). \
                    For each promising contractor, use shovels_get_contractor to check their permit \
                    history, license status, and service area. Use web_search to verify license status \
                    on the state licensing board, check reviews (BBB, Google, Yelp), and look for \
                    any complaints or disciplinary actions. Evaluate based on: active license in \
                    the correct trade, permit volume and recency in the target area, project type \
                    experience (check permit descriptions), and ratings/complaint history. Write \
                    analysis.md with a ranked shortlist (top 3-5) including license numbers, permit \
                    counts, and contact info. Create a task to request quotes from each.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "cost_estimate".into(),
                label: "Cost Estimation".into(),
                output_type: SeedOutputType::Task,
                prompt: "Develop a cost estimate for the construction project in this repository. \
                    Use shovels_search_permits in the target area filtered by project type to find \
                    comparable permitted projects and their valuations. Use web_search for current \
                    material costs (RSMeans, HomeAdvisor, local supplier pricing), labor rates \
                    for the metro area, and permit fee schedules. Break down costs into: permits \
                    and fees, site preparation, foundation, framing/structure, mechanical (HVAC, \
                    plumbing, electrical), finishes, landscaping, and contingency (10-20% depending \
                    on project complexity). Compare against per-square-foot benchmarks for the \
                    area and project type. Write estimate.md with line-item breakdown and \
                    analysis.md with comparable project data and cost drivers. Flag any items \
                    where costs are uncertain and need contractor quotes.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "code_compliance".into(),
                label: "Code Compliance".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the project plans and specifications in this repository for \
                    building code compliance. Use web_search to determine which edition of IBC/IRC \
                    the jurisdiction has adopted, plus any local amendments. Check compliance with: \
                    structural requirements (live/dead loads, wind speed, seismic category), fire \
                    safety (separation distances, fire-rated assemblies, egress), energy code \
                    (IECC edition, insulation R-values, window U-factors, HVAC efficiency), \
                    accessibility (ADA and local requirements), and zoning (setbacks, height \
                    limits, lot coverage, FAR). Use shovels_search_addresses to check parcel \
                    data for the property. Write compliance.md documenting each code section \
                    checked with pass/fail status. Create a task for each violation or unclear \
                    item that needs architect or engineer review.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

const BUILD_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous construction project agent. Research building permits, \
contractor qualifications, code requirements, and project costs using permit \
databases and web resources. Be precise with jurisdiction-specific requirements — \
building codes, permit types, and regulations vary by city and state. Do not \
fabricate permit numbers, contractor licenses, or code citations. When using \
Shovels tools, always start with shovels_get_geography to find the correct geo_id.";

const BUILD_IMPLEMENT_INSTRUCTION: &str = "\
Handle this construction task end-to-end:

0. ASSESS — identify the jurisdiction, project type, and scope. If the task is \
   missing a specific address, project description, or scope of work, signal blocked.
1. Use shovels_get_geography to find the correct geo_id for the target area.
2. Research using Shovels tools: search permits for comparable projects, look up \
   contractors with relevant specializations, check address/parcel data.
3. Research using web_search: building department requirements, applicable codes, \
   fee schedules, inspection requirements, zoning rules.
4. Write research.md with: jurisdiction-specific requirements, comparable project data, \
   applicable codes and regulations, and source URLs.
5. Draft the deliverable (permit checklist, contractor analysis, cost estimate, or \
   compliance review) with specific citations and data points.
6. Write analysis.md summarizing: key findings, recommendations, risks, and next steps.

If the task is unclear or missing critical information, write \
{\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json.";

const BUILD_IMPLEMENT_RETRY: &str =
    "\n\nPrevious attempt failed. Error:\n```\n{ERROR}\n```\nFix the issue.";

const BUILD_REVIEW_SYSTEM: &str = "\
You are an independent construction project reviewer. You did NOT draft the \
documents — review with fresh eyes for accuracy, completeness, and \
jurisdiction-specific correctness. Fix issues directly.";

const BUILD_REVIEW_INSTRUCTION: &str = "\
Review all documents in the workspace for:
1. Jurisdiction accuracy — verify the correct building codes, permit types, and \
   local requirements are cited. Use web_search to confirm.
2. Data accuracy — verify permit data, contractor info, and cost figures against \
   Shovels tools and web sources.
3. Completeness — all required permits identified, all applicable codes checked, \
   all cost line items accounted for.
4. Practicality — recommendations are actionable, timelines are realistic, \
   cost estimates include appropriate contingency.
5. Risk identification — flag anything that could cause permit denial, project \
   delays, or cost overruns.\n\
Fix any issues directly.";

const BUILD_REVIEW_RETRY: &str =
    "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues.";
