use borg_core::{
    db::KnowledgeFile,
    types::{PhaseConfig, PhaseContext, Task},
};
use tracing::warn;

const MAX_KNOWLEDGE_FILES_IN_PROMPT: usize = 24;
const MAX_INLINE_KNOWLEDGE_FILES: usize = 3;
const MAX_INLINE_KNOWLEDGE_CHARS_TOTAL: usize = 20_000;
const MAX_INLINE_KNOWLEDGE_CHARS_PER_FILE: usize = 8_000;

/// Build the instruction string passed to any agent backend.
///
/// Composes task context, the phase instruction, an optional file listing,
/// error context from the previous attempt, and any pending user messages.
/// All backends use this so the prompt format stays consistent.
pub fn build_instruction(
    task: &Task,
    phase: &PhaseConfig,
    ctx: &PhaseContext,
    file_listing: Option<&str>,
) -> String {
    let mut s = String::new();

    let kb_files = select_relevant_knowledge_files(
        &ctx.knowledge_files,
        &format!("{} {} {}", task.title, task.description, task.task_type),
        Some(&task.mode),
        None,
        (task.project_id > 0).then_some(task.project_id),
        MAX_KNOWLEDGE_FILES_IN_PROMPT,
    );
    let kb = build_knowledge_section(&kb_files, &ctx.knowledge_dir);
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
        s.push_str("\n\n---\n\nRepository manifest:\n```\n");
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
    let visible = files.len().min(MAX_KNOWLEDGE_FILES_IN_PROMPT);
    let hidden = files.len().saturating_sub(visible);
    let mut s = String::from(
        "## Knowledge Base\nYou have access to the following knowledge files at /knowledge/:\n",
    );
    if hidden > 0 {
        s.push_str(&format!(
            "(Showing {} of {} candidate files selected for this run. Search /knowledge/ if you need additional material.)\n",
            visible,
            files.len(),
        ));
    }
    let mut inline_files_used = 0usize;
    let mut inline_chars_used = 0usize;
    for file in files.iter().take(visible) {
        let allow_inline = file.inline
            && inline_files_used < MAX_INLINE_KNOWLEDGE_FILES
            && inline_chars_used < MAX_INLINE_KNOWLEDGE_CHARS_TOTAL;
        if allow_inline {
            let path = format!("{}/{}", knowledge_dir, file.file_name);
            let content = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    warn!(path = %path, "failed to read knowledge file: {}", e);
                    String::new()
                },
            };
            let content = content.trim();
            if content.is_empty() {
                s.push_str(&format!("- **{}**", file.file_name));
                if !file.description.is_empty() {
                    s.push_str(&format!(": {}", file.description));
                }
                s.push('\n');
            } else {
                let remaining_inline =
                    MAX_INLINE_KNOWLEDGE_CHARS_TOTAL.saturating_sub(inline_chars_used);
                let file_cap = remaining_inline.min(MAX_INLINE_KNOWLEDGE_CHARS_PER_FILE);
                let (content, clipped) = truncate_chars(content, file_cap);
                s.push_str(&format!("- **{}**", file.file_name));
                if !file.description.is_empty() {
                    s.push_str(&format!(" ({})", file.description));
                }
                s.push_str(":\n```\n");
                s.push_str(content);
                if clipped {
                    s.push_str("\n[...clipped for prompt budget...]");
                }
                s.push_str("\n```\n");
                inline_files_used += 1;
                inline_chars_used += content.chars().count();
            }
        } else {
            s.push_str(&format!("- `/knowledge/{}`", file.file_name));
            if !file.description.is_empty() {
                s.push_str(&format!(": {}", file.description));
            }
            if file.inline {
                s.push_str(" (listed only; inline content omitted for prompt budget)");
            }
            s.push('\n');
        }
    }
    s
}

pub fn select_relevant_knowledge_files(
    files: &[KnowledgeFile],
    topic: &str,
    mode_hint: Option<&str>,
    jurisdiction_hint: Option<&str>,
    project_id: Option<i64>,
    max_files: usize,
) -> Vec<KnowledgeFile> {
    if files.is_empty() || max_files == 0 {
        return Vec::new();
    }

    let topic_tokens = tokenize_query(topic);
    let jurisdiction_hint = jurisdiction_hint.unwrap_or("").trim().to_ascii_lowercase();
    let mode_hint = mode_hint.unwrap_or("").trim().to_ascii_lowercase();

    let mut scored = files
        .iter()
        .cloned()
        .map(|file| {
            let mut score = 0i64;
            let haystack = format!(
                "{} {} {} {} {}",
                file.file_name, file.description, file.tags, file.category, file.jurisdiction
            )
            .to_ascii_lowercase();

            if let Some(pid) = project_id {
                match file.project_id {
                    Some(file_pid) if file_pid == pid => score += 60,
                    Some(_) => score -= 30,
                    None => score += 2,
                }
            } else if file.project_id.is_none() {
                score += 2;
            }

            if !jurisdiction_hint.is_empty() {
                let file_jurisdiction = file.jurisdiction.trim().to_ascii_lowercase();
                if !file_jurisdiction.is_empty() {
                    if file_jurisdiction == jurisdiction_hint {
                        score += 20;
                    } else if file_jurisdiction.contains(&jurisdiction_hint)
                        || jurisdiction_hint.contains(&file_jurisdiction)
                    {
                        score += 10;
                    }
                }
            }

            for token in &topic_tokens {
                if haystack.contains(token) {
                    score += 5;
                }
            }

            if mode_hint == "lawborg" || mode_hint == "legal" {
                match file.category.as_str() {
                    "template" | "clause" | "policy" | "reference" => score += 8,
                    _ => score += 2,
                }
            } else if mode_hint == "sweborg" || mode_hint == "swe" {
                if file.category == "reference" {
                    score += 6;
                }
            }

            if file.inline && file.size_bytes > 200_000 {
                score -= 2;
            }

            (score, file)
        })
        .collect::<Vec<_>>();

    scored.sort_by(|(score_a, file_a), (score_b, file_b)| {
        score_b
            .cmp(score_a)
            .then_with(|| {
                file_a
                    .project_id
                    .is_some()
                    .cmp(&file_b.project_id.is_some())
                    .reverse()
            })
            .then_with(|| file_a.file_name.cmp(&file_b.file_name))
    });

    let mut out = Vec::new();
    for (score, file) in scored.into_iter() {
        if out.len() >= max_files {
            break;
        }
        if score < 0 {
            continue;
        }
        out.push(file);
    }
    out
}

fn tokenize_query(input: &str) -> Vec<String> {
    input
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .map(str::trim)
        .filter(|t| t.len() >= 2)
        .take(24)
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn truncate_chars(input: &str, max_chars: usize) -> (&str, bool) {
    if input.chars().count() <= max_chars {
        return (input, false);
    }
    let idx = input
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(input.len());
    (&input[..idx], true)
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
        "vendor_check" => Some(SKILL_VENDOR_CHECK),
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

const SKILL_VENDOR_CHECK: &str = "\
### Vendor Agreement Status Methodology

Check the status of existing agreements with a vendor. Provides a consolidated view of the legal relationship.

**Search order (by priority):** CLM/practice management (all contracts — active, expired, in negotiation), \
CRM (account status, relationship type), email (contract-related, last 6 months), \
documents (executed agreements, redlines, due diligence), chat (recent mentions, last 3 months).

**For each agreement found:** Agreement type (NDA, MSA, SOW, DPA, SLA, License), status (Active/Expired/In Negotiation/Pending Sig), \
effective date, expiration date, auto-renewal terms (renewal period, notice period), key terms (liability cap, governing law, termination), amendments.

**Gap analysis:** Check for: NDA, MSA, DPA, SOW(s), SLA, Insurance Certificate. \
Flag gaps based on relationship type (e.g., MSA present but no DPA and vendor handles personal data).

**Key alerts:** Approaching expirations within 90 days, expired agreements with surviving obligations \
(confidentiality, indemnification), required agreements not yet in place.";
