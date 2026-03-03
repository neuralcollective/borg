use borg_core::{
    db::KnowledgeFile,
    types::{PhaseConfig, PhaseContext, Task},
};

/// Build the instruction string passed to any agent backend.
///
/// Composes task context, the phase instruction, an optional file listing,
/// error context from the previous attempt, and any pending user messages.
/// All backends use this so the prompt format stays consistent.
pub fn build_instruction(task: &Task, phase: &PhaseConfig, ctx: &PhaseContext, file_listing: Option<&str>) -> String {
    let mut s = String::new();

    let kb = build_knowledge_section(&ctx.knowledge_files, &ctx.knowledge_dir);
    if !kb.is_empty() {
        s.push_str(&kb);
        s.push_str("\n\n---\n\n");
    }

    if let Some(repo_prompt) = read_repo_prompt(ctx) {
        s.push_str("## Project Context\n\n");
        s.push_str(&repo_prompt);
        s.push_str("\n\n---\n\n");
    }

    if phase.include_task_context {
        s.push_str(&format!(
            "Task: {}\n\n{}\n\n---\n\n",
            task.title, task.description
        ));
    }

    if (task.mode == "lawborg" || task.mode == "legal") && !task.task_type.is_empty() {
        if let Some(skill) = legal_skill_for_task_type(&task.task_type) {
            s.push_str("## Methodology\n\n");
            s.push_str(skill);
            s.push_str("\n\n---\n\n");
        }
    }

    s.push_str(&phase.instruction);

    if let Some(files) = file_listing.filter(|f| !f.is_empty()) {
        s.push_str("\n\n---\n\nFiles in repository:\n```\n");
        s.push_str(files);
        s.push_str("```\n");
    }

    if !task.last_error.is_empty() && !phase.error_instruction.is_empty() {
        s.push('\n');
        s.push_str(&phase.error_instruction.replace("{ERROR}", &task.last_error));
    }

    if !ctx.prior_research.is_empty() {
        s.push_str("\n\n---\n\n## Prior Research (from knowledge graph)\nThe following relevant excerpts were found from prior tasks. Use them to avoid duplicating research:\n\n");
        for (i, chunk) in ctx.prior_research.iter().enumerate() {
            s.push_str(&format!("{}. {}\n\n", i + 1, chunk));
        }
    }

    if !ctx.pending_messages.is_empty() {
        let is_revision = ctx.revision_count > 0;
        if is_revision {
            s.push_str(&format!(
                "\n\n---\n## Revision #{} — Reviewer Feedback\n\
                 Your previous draft was reviewed and changes were requested. \
                 Address ALL of the following feedback points specifically:\n",
                ctx.revision_count
            ));
            for (role, content) in &ctx.pending_messages {
                s.push_str(&format!("\n**[{}]**: {}\n", role, content));
            }
            s.push_str(
                "\nIMPORTANT: Focus on the reviewer's feedback. Do not rewrite sections that were not flagged. \
                 Make targeted, precise changes that directly address each feedback point.\n"
            );
        } else {
            s.push_str("\n\n---\nThe following messages were sent by the user or director while this task was queued:\n");
            for (role, content) in &ctx.pending_messages {
                s.push_str(&format!("\n[{}]: {}", role, content));
            }
        }
    }

    s
}

/// Build the `## Knowledge Base` section prepended to agent instructions.
pub fn build_knowledge_section(files: &[KnowledgeFile], knowledge_dir: &str) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## Knowledge Base\nYou have access to the following knowledge files at /knowledge/:\n",
    );
    for file in files {
        if file.inline {
            let path = format!("{}/{}", knowledge_dir, file.file_name);
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let content = content.trim();
            if content.is_empty() {
                s.push_str(&format!("- **{}**", file.file_name));
                if !file.description.is_empty() {
                    s.push_str(&format!(": {}", file.description));
                }
                s.push('\n');
            } else {
                s.push_str(&format!("- **{}**", file.file_name));
                if !file.description.is_empty() {
                    s.push_str(&format!(" ({})", file.description));
                }
                s.push_str(":\n```\n");
                s.push_str(content);
                s.push_str("\n```\n");
            }
        } else {
            s.push_str(&format!("- `/knowledge/{}`", file.file_name));
            if !file.description.is_empty() {
                s.push_str(&format!(": {}", file.description));
            }
            s.push('\n');
        }
    }
    s
}

/// Read the per-repo prompt from the explicit prompt_file config, or by
/// auto-detecting `.borg/prompt.md` in the work dir / repo root.
fn read_repo_prompt(ctx: &PhaseContext) -> Option<String> {
    use borg_core::ipc::{self, IpcReadResult};

    // 1. Explicit prompt_file from config (operator-trusted absolute path)
    if !ctx.repo_config.prompt_file.is_empty() {
        if let IpcReadResult::Ok(content) = ipc::read_trusted_path(&ctx.repo_config.prompt_file) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }

    // 2. .borg/prompt.md in the work dir (may differ from repo root during tasks)
    if let IpcReadResult::Ok(content) = ipc::read_file(&ctx.work_dir, ".borg/prompt.md") {
        let trimmed = content.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    // 3. .borg/prompt.md in the repo root (skip if same path as work dir)
    if ctx.repo_config.path != ctx.work_dir {
        if let IpcReadResult::Ok(content) = ipc::read_file(&ctx.repo_config.path, ".borg/prompt.md")
        {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }

    None
}

/// Returns a condensed skill/methodology block for a given legal task type.
/// Based on Anthropic's knowledge-work-plugins legal skills.
fn legal_skill_for_task_type(task_type: &str) -> Option<&'static str> {
    match task_type {
        "contract_analysis" | "contract_review" => Some(SKILL_CONTRACT_REVIEW),
        "nda_triage" | "nda" => Some(SKILL_NDA_TRIAGE),
        "compliance" | "regulatory_analysis" => Some(SKILL_COMPLIANCE),
        "demand_letter" | "motion_brief" => Some(SKILL_RISK_ASSESSMENT),
        "canned_response" | "template_response" => Some(SKILL_CANNED_RESPONSES),
        "meeting_briefing" | "briefing" => Some(SKILL_MEETING_BRIEFING),
        "risk_assessment" => Some(SKILL_RISK_ASSESSMENT),
        _ => None,
    }
}

const SKILL_CONTRACT_REVIEW: &str = "\
### Contract Review Methodology

Review commercial contracts against the organization's negotiation playbook. For each clause, classify severity:

**GREEN** — Aligns with or better than standard position. No negotiation needed.
**YELLOW** — Outside standard but within negotiable range. Provide specific redline language + fallback position.
**RED** — Material risk requiring senior counsel escalation. Include market position, exposure, and escalation path.

**Minimum clause coverage:** Limitation of Liability (caps, carveouts, consequential damages), \
Indemnification (scope, mutuality, cap, IP/data breach), IP Ownership (pre-existing, developed, work-for-hire, license grants), \
Data Protection (DPA, processing terms, sub-processors, breach notification, cross-border), \
Confidentiality (scope, term, carveouts, return/destruction), Representations & Warranties, \
Term & Termination (renewal, convenience, cause, wind-down), Governing Law & Dispute Resolution, \
Insurance, Assignment, Force Majeure, Payment Terms.

**Negotiation priority tiers:**
- Tier 1 (Must-Haves): Deal-breakers requiring resolution before proceeding
- Tier 2 (Should-Haves): Strong preferences with flexibility
- Tier 3 (Nice-to-Haves): Improvements that can be strategically conceded

**Redline format:** Clause reference, current language (quoted), proposed alternative, rationale, priority, fallback positions.";

const SKILL_NDA_TRIAGE: &str = "\
### NDA Triage Methodology

Rapidly triage incoming NDAs against screening criteria. Classify for routing:

**GREEN** (Standard Approval) — All baseline criteria met. Can proceed without counsel review.
**YELLOW** (Counsel Review) — Minor deviations. Examples: broader definition, longer term, narrow residuals, minor jurisdiction issue.
**RED** (Significant Issues) — Material deviations. Examples: missing critical carveouts, embedded non-compete/non-solicit, unreasonable term (10+ years), IP assignment.

**10 screening dimensions:** (1) Mutual vs. unilateral structure, (2) Confidential information definition scope, \
(3) Party obligations, (4) Standard carveouts (public knowledge, prior possession, independent development, legal compulsion), \
(5) Permitted disclosures, (6) Term duration (standard 1-3 years, 2-5 year survival), \
(7) Return/destruction provisions, (8) Remedies, (9) Problematic provisions (non-compete → RED, IP assignment → RED, \
liquidated damages → RED), (10) Governing law.

**Key rule:** If the document contains substantive commercial terms beyond NDA scope, flag RED and recommend full contract review.";

const SKILL_COMPLIANCE: &str = "\
### Compliance Review Methodology

Navigate privacy regulations (GDPR, CCPA/CPRA, LGPD, POPIA, PIPEDA, PDPA, Privacy Act, PIPL, UK GDPR).

**DPA review checklist:** Required elements, processor obligations, international transfers (SCCs, adequacy decisions, \
BCRs), sub-processor controls, data breach notification (72h GDPR, reasonable CCPA), audit rights.

**Data subject request handling:**
- GDPR: 30 days response, free of charge, right to access/erasure/portability/rectification/restriction/objection
- CCPA/CPRA: 45 days response, right to know/delete/opt-out/correct/limit sensitive data use
- Track applicable exemptions (legal hold, legal obligation, ongoing litigation)

**Regulatory monitoring:** Track pending regulations, enforcement actions, significant court decisions. \
Escalation triggers: new enforcement action in relevant jurisdiction, pending regulation with <6 months to effective date, \
court decision changing compliance obligations.";

const SKILL_RISK_ASSESSMENT: &str = "\
### Legal Risk Assessment Methodology

Assess and classify legal risks using a severity-by-likelihood matrix:

**Severity (1-5):** 1=Negligible, 2=Minor, 3=Moderate (potential financial exposure 5-15% of value), \
4=Major (significant exposure 15-25%), 5=Critical (major exposure >25%, significant reputational damage)

**Likelihood (1-5):** 1=Remote (highly unlikely), 2=Unlikely, 3=Possible, 4=Probable, 5=Almost Certain

**Risk Score = Severity × Likelihood:**
- 1-4 → GREEN (Low): Accept, document, monitor quarterly
- 5-9 → YELLOW: Mitigate, monitor monthly, assign owner, brief stakeholders
- 10-15 → ORANGE: Escalate to senior counsel, develop mitigation plan, weekly review
- 16-25 → RED (Critical): Immediate C-suite escalation, engage outside counsel, preserve evidence, daily monitoring

**Documentation:** Each risk assessment must include: description, background, severity/likelihood analysis, \
contributing/mitigating factors, mitigation options table, recommended approach, residual risk, \
monitoring plan with specific next steps and owners.";

const SKILL_CANNED_RESPONSES: &str = "\
### Template Response Methodology

Generate responses from configured templates for routine legal inquiries. Categories:
- Data Subject Requests (acknowledgments, verifications, fulfillment, denials)
- Discovery Holds (initial notices, reminders, modifications, releases)
- Privacy Inquiries (cookies, policies, data sharing)
- Vendor Legal Questions (contracts, amendments, compliance, audits)
- NDA Requests (standard forms, counterparty NDAs, declines, renewals)
- Subpoena/Legal Process (acknowledgments, objections, extensions)
- Insurance Notifications (claims, supplemental information)

**Critical escalation triggers (always flag):** Matters involving litigation, regulatory investigations, \
binding commitments, criminal liability, media attention, or unprecedented situations.

**Customization requirements:** Every response must include correct names, dates, jurisdictional regulations, \
applicable deadlines, and appropriate signatures. Present draft for review before sending.

**Template lifecycle:** Creation → Review → Publication → Use → Feedback → Update → Retirement.";

const SKILL_MEETING_BRIEFING: &str = "\
### Meeting Briefing Methodology

Generate contextual briefings. Three modes: daily brief, topic brief, incident brief.

**Briefing structure:** Meeting details, participants table, agenda, background context, key documents, \
open issues, legal considerations, talking points, questions, decisions needed, red lines, prior follow-ups, preparation gaps.

**Meeting-type guidance:**
- Deal reviews: Counterparty dynamics, approval requirements, key term positions
- Board meetings: Risk highlights, regulatory updates, litigation summaries
- Regulatory meetings: Compliance posture, privilege considerations

**Action item requirements:** Specific (not vague), each with owner, deadline, and priority level. \
Follow-up cadence: High=daily, Medium=weekly, Low=monthly.

**Incident briefs (urgent):** Speed over completeness. Flag litigation hold and preservation obligations immediately. \
Note privilege considerations. For data breaches: flag notification deadlines (72h GDPR). \
Recommend outside counsel for significant matters.";
