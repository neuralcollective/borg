use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, setup_phase};

pub fn data_mode() -> PipelineMode {
    PipelineMode {
        name: "databorg".into(),
        label: "Data Analysis".into(),
        category: "Engineering".into(),
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
                commit_message: "data: analysis from databorg agent".into(),
                error_instruction: DATA_IMPLEMENT_RETRY.into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    DATA_IMPLEMENT_SYSTEM,
                    DATA_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,Bash,web_search,WebFetch",
                    "review",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "review: revisions from data review agent".into(),
                fresh_session: true,
                error_instruction: DATA_REVIEW_RETRY.into(),
                ..agent_phase(
                    "review",
                    "Review",
                    DATA_REVIEW_SYSTEM,
                    DATA_REVIEW_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,Bash",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "quality".into(),
                label: "Data Quality".into(),
                output_type: SeedOutputType::Task,
                prompt: "Audit the datasets in this repository for quality issues. Check for: \
                    missing values and their patterns (random vs systematic), duplicate records, \
                    inconsistent formats (dates, currencies, identifiers), outliers beyond \
                    reasonable bounds, referential integrity between related tables, encoding \
                    issues, and schema drift between files. Quantify each issue (e.g. '15% of \
                    rows missing zip code'). Create a task for each issue with the specific \
                    file, column, and recommended fix."
                    .into(),
                allowed_tools: "Read,Glob,Grep,Bash".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "pipeline".into(),
                label: "Pipeline Review".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the data pipeline code in this repository. Check for: missing \
                    error handling on data ingestion, transformations that silently drop rows, \
                    hardcoded paths or credentials, missing data validation at boundaries, \
                    inefficient operations (loading entire datasets when filtering would suffice), \
                    missing logging/observability on pipeline stages, and missing idempotency \
                    (will a re-run produce correct results?). Create a task for each concrete \
                    issue with the file path and recommended fix."
                    .into(),
                allowed_tools: "Read,Glob,Grep,Bash".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "insights".into(),
                label: "Insight Discovery".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Explore the datasets in this repository for actionable insights. \
                    Look at distributions, correlations, trends over time, segmentation \
                    opportunities, and anomalies worth investigating. Each proposal should \
                    describe: the specific finding, the data that supports it, and what \
                    action it suggests. Base proposals on actual data you read, not \
                    hypothetical analysis."
                    .into(),
                allowed_tools: "Read,Glob,Grep,Bash".into(),
                target_primary_repo: false,
            },
        ],
    }
}

const DATA_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous data analysis agent. Explore datasets, build pipelines, \
write queries, and produce analyses end-to-end. Be precise with numbers — \
always show your methodology and verify aggregations. Use Bash for data \
exploration (python, psql, jq, csvkit, etc.). Do not fabricate data or statistics.";

const DATA_IMPLEMENT_INSTRUCTION: &str = "\
Handle this data task end-to-end:
1. Explore the available data: file formats, schemas, row counts, value distributions
2. Plan your analysis approach and document it in methodology.md
3. Implement the analysis, pipeline, or transformation
4. Validate results: spot-check aggregations, verify row counts, sanity-check outputs
5. Write findings.md with results, visualisation suggestions, and next steps

If the task is unclear or data is missing, write \
{\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json.";

const DATA_IMPLEMENT_RETRY: &str =
    "\n\nPrevious attempt failed. Error:\n```\n{ERROR}\n```\nFix the issue.";

const DATA_REVIEW_SYSTEM: &str = "\
You are an independent data review agent. You did NOT perform the analysis — \
review it with fresh eyes for correctness, methodology, and completeness. \
Fix any issues directly.";

const DATA_REVIEW_INSTRUCTION: &str = "\
Review all analysis output in the workspace for:
1. Methodology — are aggregations correct? Sample sizes adequate? Joins valid?
2. Data quality — were nulls, duplicates, and outliers handled appropriately?
3. Reproducibility — can the analysis be re-run from the documented steps?
4. Conclusions — are findings supported by the data shown? Any overreach?
5. Completeness — are there obvious follow-up questions left unaddressed?\n\
Fix any issues directly.";

const DATA_REVIEW_RETRY: &str =
    "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues.";
