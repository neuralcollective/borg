//! Borgbench integration for lawborg self-improvement.
//!
//! Translates borgbench matter packs into lawborg tasks, collects results,
//! and feeds scorecards back for prompt tuning. The hidden evaluation
//! materials (rubrics, answer banks, evidence maps) never enter this module —
//! scoring is handled entirely by borgbench's scorer.

use std::path::Path;

use anyhow::{Context, Result};

/// Scorecard fields borg is allowed to see (its own graded results).
/// This struct intentionally omits rubric content, evidence maps, and
/// answer bank details — borg sees scores and rationales, not the test.
#[derive(Debug, serde::Deserialize)]
pub struct BenchScorecard {
    pub run_id: String,
    pub matter_id: String,
    pub hard_fail: bool,
    pub hard_fail_reasons: Vec<String>,
    pub composite_score: f64,
    pub pass_status: String,
    pub dimensions: Vec<BenchDimension>,
    #[serde(default)]
    pub missed_facts: Vec<String>,
    #[serde(default)]
    pub missed_issues: Vec<String>,
    #[serde(default)]
    pub red_flags_triggered: Vec<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct BenchDimension {
    pub id: String,
    pub raw_score: u8,
    pub weighted_score: f64,
    pub rationale: String,
}

/// Weakness identified from scorecard analysis. Used by the tuning loop
/// to decide which lawborg prompts to adjust.
#[derive(Debug)]
pub struct Weakness {
    pub dimension: String,
    pub raw_score: u8,
    pub rationale: String,
}

/// Read a borgbench matter brief and build a lawborg task description.
/// Only reads from visible/ — never touches hidden/.
pub fn matter_to_task_description(matter_dir: &Path) -> Result<String> {
    let brief_path = matter_dir.join("visible/brief.md");
    let brief = std::fs::read_to_string(&brief_path)
        .with_context(|| format!("reading brief at {}", brief_path.display()))?;

    let spec_path = matter_dir.join("visible/deliverable_spec.json");
    let spec = std::fs::read_to_string(&spec_path)
        .with_context(|| format!("reading deliverable spec at {}", spec_path.display()))?;

    let corpus_dir = matter_dir.join("visible/corpus");
    let mut corpus_listing = Vec::new();
    if corpus_dir.is_dir() {
        for entry in std::fs::read_dir(&corpus_dir)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e == "md") {
                corpus_listing.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        corpus_listing.sort();
    }

    Ok(format!(
        "{brief}\n\n---\n\n## Deliverable Specification\n\n```json\n{spec}\n```\n\n\
         ## Corpus Documents\n\n{}\n\n\
         The corpus documents are in the `visible/corpus/` directory. Read them all.",
        corpus_listing
            .iter()
            .map(|f| format!("- `{f}`"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

/// Extract weaknesses from a scorecard, sorted worst-first.
pub fn extract_weaknesses(sc: &BenchScorecard) -> Vec<Weakness> {
    let mut weak: Vec<Weakness> = sc
        .dimensions
        .iter()
        .map(|d| Weakness {
            dimension: d.id.clone(),
            raw_score: d.raw_score,
            rationale: d.rationale.clone(),
        })
        .collect();
    weak.sort_by_key(|w| w.raw_score);
    weak
}

/// Build a tuning prompt that tells the agent what to improve, based on
/// scorecard feedback. Does NOT reveal rubric content — only the agent's
/// own scores and the grader's rationale for those scores.
pub fn tuning_prompt(weaknesses: &[Weakness], hard_fail_reasons: &[String]) -> String {
    let mut parts = vec![
        "Your previous benchmark run was graded. Here are the results:\n".to_string(),
    ];

    if !hard_fail_reasons.is_empty() {
        parts.push(format!(
            "HARD FAILS (automatic failure):\n{}\n",
            hard_fail_reasons
                .iter()
                .map(|r| format!("- {r}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    parts.push("DIMENSION SCORES (weakest first):\n".to_string());
    for w in weaknesses {
        parts.push(format!("- {} ({}/5): {}", w.dimension, w.raw_score, w.rationale));
    }

    parts.push(
        "\nBased on this feedback, identify the specific changes to your legal analysis \
         approach that would address these weaknesses. Focus on the lowest-scoring \
         dimensions and any hard fails first."
            .to_string(),
    );

    parts.join("\n")
}
