use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, setup_phase};

pub fn health_mode() -> PipelineMode {
    PipelineMode {
        name: "healthborg".into(),
        label: "Healthcare Admin".into(),
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
                commit_message: "health: research and draft from healthborg agent".into(),
                error_instruction: HEALTH_IMPLEMENT_RETRY.into(),
                system_prompt: HEALTH_IMPLEMENT_SYSTEM.into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    "",
                    HEALTH_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,web_search,WebFetch",
                    "review",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "review: revisions from health review agent".into(),
                fresh_session: true,
                error_instruction: HEALTH_REVIEW_RETRY.into(),
                system_prompt: HEALTH_REVIEW_SYSTEM.into(),
                ..agent_phase(
                    "review",
                    "Review",
                    "",
                    HEALTH_REVIEW_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,web_search,WebFetch",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "insurance_appeal".into(),
                label: "Insurance Appeal".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the denial documentation in this repository. Identify the denied service, \
                    denial reason code, and plan type (employer, ACA marketplace, Medicaid, Medicare). \
                    Research applicable regulations: use federal_register_search for CMS coverage \
                    determinations and Medicare rules. Use openstates_search_bills for state insurance \
                    mandates, external review laws, and prompt payment statutes. Use web_search for \
                    current clinical guidelines (ACR, NCCN, AMA) that support medical necessity. \
                    Check if the denial triggers external review rights under ACA Section 2719. \
                    Draft an appeal letter citing: specific plan provision that covers the service, \
                    clinical guidelines supporting medical necessity, applicable state/federal regulations \
                    requiring coverage, and procedural errors in the denial if any. \
                    Write research.md with regulatory findings and analysis.md with appeal strategy. \
                    Create a task for each missing document the patient needs to provide.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "prior_auth".into(),
                label: "Prior Authorization".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the clinical documentation and payer requirements in this repository. \
                    Identify the procedure (CPT/HCPCS codes), diagnosis (ICD-10 codes), and payer. \
                    Use web_search to find the payer's specific prior auth criteria, clinical policy \
                    bulletins, and required documentation checklists. Use federal_register_search for \
                    CMS National Coverage Determinations and Local Coverage Determinations if Medicare. \
                    Prepare the prior authorization submission: letter of medical necessity linking \
                    diagnosis to proposed treatment, supporting clinical guidelines, relevant lab results \
                    or imaging findings from the clinical notes, and peer-reviewed literature if the \
                    treatment is non-standard. Include all required payer form fields. \
                    Create a task for each gap in clinical documentation that the provider needs to supply.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "bill_review".into(),
                label: "Medical Bill Review".into(),
                output_type: SeedOutputType::Task,
                prompt: "Audit the medical bills and EOB documents in this repository for errors. \
                    Check for: upcoding (charges billed at higher complexity than documented), \
                    unbundling (separately billing components that should be a single code), \
                    duplicate charges for the same service/date, balance billing violations \
                    (out-of-network provider billing beyond allowed amount), charges exceeding \
                    usual and customary rates, and services not documented in clinical notes. \
                    Use web_search for current CPT code descriptions and Medicare fee schedules. \
                    Use federal_register_search for No Surprises Act provisions and CMS billing rules. \
                    Use openstates_search_bills for state balance billing protections and surprise \
                    billing laws. Write research.md with findings and analysis.md with recommended \
                    disputes. Create a task for each billing error with the specific code, charge, \
                    and regulation it violates.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

const HEALTH_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous healthcare administration agent. Handle insurance appeals, \
prior authorizations, and medical bill disputes end-to-end. Research applicable \
regulations, draft administrative documents, and cite specific plan provisions, \
clinical guidelines, and state/federal laws. Be precise with codes (CPT, ICD-10, \
denial reason codes) and regulatory citations. Do not fabricate clinical information \
or regulatory references — verify with web search and official sources.";

const HEALTH_IMPLEMENT_INSTRUCTION: &str = "\
Handle this healthcare admin task end-to-end:

0. ASSESS — check if you have enough context. If the task is missing plan details, \
   denial reason, procedure codes, diagnosis codes, or state jurisdiction, signal \
   blocked and ask the user rather than guessing.
1. Research applicable regulations — federal (ACA, ERISA, CMS rules, No Surprises Act) \
   and state (insurance mandates, external review laws, balance billing protections). \
   Use federal_register_search for CMS rules, openstates_search_bills for state law, \
   web_search for payer-specific policies and clinical guidelines.
2. Write research.md with: regulatory framework, applicable plan provisions, \
   clinical guidelines supporting the position, and source URLs for everything.
3. Draft the administrative document (appeal letter, prior auth submission, \
   or billing dispute) with proper formatting. Cite specific: plan section numbers, \
   regulation citations (42 CFR, state insurance code), clinical guideline references, \
   and procedure/diagnosis codes.
4. Write analysis.md summarizing: strategy, key regulatory arguments, \
   likelihood of success, escalation options (external review, state insurance \
   commissioner, CMS complaint), and timeline expectations.

If the task is unclear or missing critical information, write \
{\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json.\n\
If the task is already resolved or not actionable, write \
{\"status\":\"abandon\",\"reason\":\"...\"} to .borg/signal.json.";

const HEALTH_IMPLEMENT_RETRY: &str =
    "\n\nPrevious attempt failed. Error:\n```\n{ERROR}\n```\nFix the issue.";

const HEALTH_REVIEW_SYSTEM: &str = "\
You are an independent healthcare admin reviewer. You did NOT draft the documents — \
review them with fresh eyes for regulatory accuracy, completeness, and persuasiveness. \
Fix any issues you find directly. Ensure all regulatory citations are correct and \
all clinical arguments are supported by cited guidelines.";

const HEALTH_REVIEW_INSTRUCTION: &str = "\
Review all documents in the workspace for:
1. Regulatory accuracy — verify cited federal and state regulations are current and \
   correctly applied. Use federal_register_search and openstates_search_bills to confirm.
2. Clinical support — verify that cited clinical guidelines actually support the \
   medical necessity argument. Use web_search for current guideline versions.
3. Code accuracy — verify CPT, ICD-10, and denial reason codes are correct and \
   consistently used throughout.
4. Completeness — all required elements present (plan citations, regulatory basis, \
   clinical justification, procedural arguments).
5. Tone — professional, factual, assertive but not adversarial. Appropriate for \
   submission to a payer or regulatory body.
6. Escalation path — confirm the analysis.md includes viable next steps if the \
   initial submission is denied.
7. Deadlines — flag any filing deadlines that are approaching or may have been missed.\n\
Fix any issues directly. Do not just list problems — resolve them.";

const HEALTH_REVIEW_RETRY: &str =
    "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues.";
