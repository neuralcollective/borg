use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, setup_phase};

pub fn medwrite_mode() -> PipelineMode {
    PipelineMode {
        name: "medborg".into(),
        label: "Medical Writing".into(),
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
                commit_message: "medwrite: research and draft from medborg agent".into(),
                error_instruction: MEDWRITE_IMPLEMENT_RETRY.into(),
                system_prompt: MEDWRITE_IMPLEMENT_SYSTEM.into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    "",
                    MEDWRITE_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,web_search,WebFetch",
                    "review",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "review: revisions from medwrite review agent".into(),
                fresh_session: true,
                error_instruction: MEDWRITE_REVIEW_RETRY.into(),
                system_prompt: MEDWRITE_REVIEW_SYSTEM.into(),
                ..agent_phase(
                    "review",
                    "Review",
                    "",
                    MEDWRITE_REVIEW_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,web_search,WebFetch",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "csr_draft".into(),
                label: "Clinical Study Report".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the clinical data and protocol documents in this repository. \
                    Draft a Clinical Study Report (CSR) following ICH E3 structure: \
                    title page, synopsis, table of contents, ethics, investigators, study plan, \
                    study patients, efficacy evaluation, safety evaluation, discussion, and conclusions. \
                    Use web_search to verify current ICH E3 guidance and any therapeutic-area-specific \
                    reporting requirements. Cross-reference CONSORT for RCTs or STROBE for observational \
                    studies. Create a task for each data gap or missing analysis.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "regulatory_submission".into(),
                label: "Regulatory Submission".into(),
                output_type: SeedOutputType::Task,
                prompt: "Analyze the documents in this repository to prepare a regulatory submission. \
                    Determine the submission type (IND, NDA, BLA, MAA, 510(k), PMA, CE marking). \
                    Use web_search for current FDA/EMA guidance documents and ICH M4 CTD format. \
                    Draft the relevant Module 2 summaries (Quality Overall Summary, Nonclinical Overview, \
                    Clinical Overview, Clinical Summary) or device-specific summaries as applicable. \
                    Create a task for each missing section or document required for the submission.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "lit_review".into(),
                label: "Literature Review".into(),
                output_type: SeedOutputType::Task,
                prompt: "Conduct a systematic literature review on the topic defined in this repository. \
                    Use web_search to search PubMed, Google Scholar, and Cochrane Library for relevant \
                    publications. Follow PRISMA guidelines for systematic reviews or PRISMA-ScR for \
                    scoping reviews. Document the search strategy (databases, terms, filters, date range), \
                    screening criteria (inclusion/exclusion), and results. Draft a PRISMA flow diagram \
                    description, evidence summary tables, and narrative synthesis. Create a task for \
                    each full-text article that needs manual review or data extraction.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "manuscript".into(),
                label: "Manuscript Preparation".into(),
                output_type: SeedOutputType::Task,
                prompt: "Prepare a scientific manuscript from the data and analysis in this repository. \
                    Identify the target journal and retrieve its author guidelines via web_search. \
                    Follow ICMJE recommendations and GPP3 (Good Publication Practice) guidelines. \
                    Apply the appropriate reporting guideline (CONSORT for RCTs, STROBE for observational, \
                    PRISMA for reviews, ARRIVE for animal studies, CARE for case reports). \
                    Draft: title, structured abstract, introduction, methods, results, discussion, \
                    conclusions, references (Vancouver/NLM style). Create a task for each figure, \
                    table, or supplementary material that needs to be produced.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "pharmacovigilance".into(),
                label: "Pharmacovigilance Report".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the safety data in this repository and draft the appropriate \
                    pharmacovigilance document. Determine the report type: PBRER/PSUR (periodic \
                    benefit-risk), DSUR (development safety update), or RMP (risk management plan). \
                    Use web_search for current ICH E2C(R2) guidance for PBRER, ICH E2F for DSUR, \
                    and GVP Module V for RMP. Analyze adverse event data, signal detection results, \
                    and benefit-risk assessment. Create a task for each data source that needs \
                    querying or each safety signal that needs further evaluation.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "patient_docs".into(),
                label: "Patient Documents".into(),
                output_type: SeedOutputType::Task,
                prompt: "Draft patient-facing documents based on the clinical information in this repository. \
                    This may include: informed consent forms (ICF), patient information leaflets (PIL), \
                    medication guides, or patient-reported outcome questionnaires. Use web_search for \
                    current FDA guidance on informed consent (21 CFR 50) and EMA guidance on PILs. \
                    Write at a 6th-8th grade reading level using plain language principles. \
                    Include all required elements per ICH E6(R2) GCP for informed consent. \
                    Create a task for each document variant needed (e.g., per-site amendments, \
                    translations, age-appropriate versions for pediatric studies).".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

const MEDWRITE_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous medical writing agent. You produce regulatory-grade medical \
and scientific documents following ICH guidelines, FDA/EMA requirements, and \
established reporting standards (CONSORT, STROBE, PRISMA, etc.). You write with \
scientific precision, using proper medical terminology and citation practices. \
Never fabricate data, statistics, references, or clinical results. Every claim \
must be traceable to source data in the repository or verified published literature. \
Use Vancouver/NLM citation format unless the task specifies otherwise.";

const MEDWRITE_IMPLEMENT_INSTRUCTION: &str = "\
Handle this medical writing task end-to-end:

0. ASSESS — check if you have enough context. If the task is missing critical \
   information (therapeutic area, study design, target audience, regulatory jurisdiction, \
   target journal, or submission type), signal blocked and ask the user rather than guessing.
1. Research — use web_search to find current regulatory guidance (FDA guidances, EMA \
   guidelines, ICH guidelines), reporting standards (CONSORT, STROBE, PRISMA, ARRIVE, \
   CARE), and relevant published literature on PubMed/Google Scholar. Verify that \
   guidance documents are current versions.
2. Write research.md with: applicable guidelines and their versions, reporting standard \
   checklist items, key references found, and regulatory requirements specific to \
   the document type and jurisdiction.
3. Draft the document following the appropriate structure:
   - CSR: ICH E3 structure (synopsis, study plan, patients, efficacy, safety, discussion)
   - Regulatory submission: ICH M4 CTD format (Module 2 summaries)
   - Manuscript: IMRAD (Introduction, Methods, Results, Discussion)
   - Literature review: PRISMA flow + evidence tables + narrative synthesis
   - Patient documents: plain language, 6th-8th grade level, all required consent elements
   - PV reports: ICH E2C(R2) for PBRER, ICH E2F for DSUR
4. Ensure all statistical claims match source data in the repository. Flag any \
   discrepancies between data files and narrative text.
5. Apply proper citation format (Vancouver/NLM default). Every referenced study \
   must include PMID, DOI, or URL.
6. Write analysis.md summarizing: document structure rationale, key regulatory \
   requirements addressed, outstanding gaps, and recommended next steps.

If the task is unclear or missing critical information, write \
{\"status\":\"blocked\",\"reason\":\"...\"} to .borg/signal.json.\n\
If the task is already resolved or not actionable, write \
{\"status\":\"abandon\",\"reason\":\"...\"} to .borg/signal.json.";

const MEDWRITE_IMPLEMENT_RETRY: &str =
    "\n\nPrevious attempt failed. Error:\n```\n{ERROR}\n```\nFix the issue.";

const MEDWRITE_REVIEW_SYSTEM: &str = "\
You are an independent medical writing reviewer. You did NOT draft the documents — \
review them with fresh eyes for scientific accuracy, regulatory compliance, and \
completeness. Fix any issues you find directly. You have deep expertise in ICH \
guidelines, GCP, biostatistics, and medical publishing standards.";

const MEDWRITE_REVIEW_INSTRUCTION: &str = "\
Review all documents in the workspace for:
1. Scientific accuracy — verify that all statistical claims, p-values, confidence \
   intervals, and effect sizes are consistent with source data in the repository. \
   Flag any numerical discrepancies.
2. Regulatory compliance — check against applicable ICH guidelines (E3, E6, M4, E2C, \
   E2F) and FDA/EMA requirements for the document type. Use web_search to confirm \
   current guidance versions.
3. Reporting standard adherence — verify compliance with the applicable checklist \
   (CONSORT, STROBE, PRISMA, etc.). Check each item. Note any missing items.
4. References — verify that cited publications exist and are accurately represented. \
   Use web_search to spot-check key references on PubMed. Confirm PMIDs and DOIs.
5. Internal consistency — cross-check numbers between tables, figures, text, and \
   abstract. Ensure the synopsis/abstract matches the full document.
6. Language and style — appropriate scientific register, consistent terminology, \
   correct abbreviation usage (defined on first use), proper units and SI conventions.
7. Completeness — all required sections present per the applicable guideline. \
   All tables and figures referenced in text. All appendices listed.
8. Plain language (for patient documents) — reading level appropriate, medical \
   jargon explained, all required informed consent elements present per 21 CFR 50 \
   and ICH E6(R2).\n\
Fix any issues directly. Do not just list problems — resolve them.";

const MEDWRITE_REVIEW_RETRY: &str =
    "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues.";
