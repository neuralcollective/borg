use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use super::courtlistener::CourtListenerClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub text: String,
    pub citation_type: String,
    pub reporter: String,
    pub volume: String,
    pub page: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub citation_text: String,
    pub citation_type: String,
    pub status: String,
    pub source: String,
    pub treatment: String,
    pub checked_at: String,
}

struct CitationPattern {
    regex: Regex,
    citation_type: &'static str,
}

static PATTERNS: LazyLock<Vec<CitationPattern>> = LazyLock::new(|| {
    vec![
        CitationPattern {
            regex: Regex::new(r"\d+\s+U\.S\.\s+\d+").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"\d+\s+S\.\s*Ct\.\s+\d+").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"\d+\s+L\.\s*Ed\.\s*(?:2d\s+)?\d+").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"\d+\s+F\.(?:\d+(?:d|th)|Supp\.(?:\s*\d+d)?|App'x)\s+\d+(?:\s*\([^)]+\d{4}\))?").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"\d+\s+(?:A\.\d+d|N\.E\.\d+d|So\.\s*\d+d|P\.\d+d|S\.E\.\d+d|N\.W\.\d+d|S\.W\.\d+d)\s+\d+").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"\d+\s+(?:Cal\.(?:\s*\d+th)?|N\.Y\.(?:S\.)?(?:\s*\d+d)?)\s+\d+").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"\[\d{4}\]\s+(?:UKSC|EWCA\s+(?:Civ|Crim)|EWHC|UKHL|UKPC|EWCOP|UKUT|UKFTT)\s+\d+").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"Case\s+[CT]-\d+/\d+").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"(?:\[\d{4}\]\s+|\d{4}\s+)(?:SCC|SCR|FC|FCA|ONCA|BCCA|ABCA|QCCA|NSCA|NBCA)\s+\d+").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"\d{4}\s+CanLII\s+\d+\s+\([A-Z]+\)").unwrap(),
            citation_type: "case",
        },
        CitationPattern {
            regex: Regex::new(r"\d+\s+U\.S\.C\.?\s+§+\s*\d+[\w-]*").unwrap(),
            citation_type: "statute",
        },
        CitationPattern {
            regex: Regex::new(r"\d+\s+C\.F\.R\.?\s+§+\s*\d+[\d.]*").unwrap(),
            citation_type: "regulation",
        },
        CitationPattern {
            regex: Regex::new(r"(?:Cal|N\.Y|Tex|Fla|Ill|Ohio|Pa|Mass|Mich|Ga|N\.J|Va|Wash|Ariz|Md|Minn|Mo|Wis|Colo|Conn|Or|S\.C|Ky|La|Okla|Ala|Ind)\.?\s+[A-Z][A-Za-z.&\s]+§+\s*\d+[\w.-]*").unwrap(),
            citation_type: "statute",
        },
    ]
});

pub fn extract_citations(markdown: &str) -> Vec<Citation> {
    let mut seen = std::collections::HashSet::new();
    let mut citations = Vec::new();

    for pattern in PATTERNS.iter() {
        for m in pattern.regex.find_iter(markdown) {
            let text = m.as_str().trim().to_string();
            if seen.contains(&text) {
                continue;
            }
            seen.insert(text.clone());

            let (reporter, volume, page) = parse_reporter_parts(&text);
            citations.push(Citation {
                text,
                citation_type: pattern.citation_type.to_string(),
                reporter,
                volume,
                page,
            });
        }
    }

    citations
}

fn parse_reporter_parts(text: &str) -> (String, String, String) {
    static REPORTER_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(\d+)\s+([A-Z][A-Za-z.\s']+?)\s+(\d+)").unwrap());

    if let Some(caps) = REPORTER_RE.captures(text) {
        (
            caps[2].trim().to_string(),
            caps[1].to_string(),
            caps[3].to_string(),
        )
    } else {
        (String::new(), String::new(), String::new())
    }
}

pub async fn verify_citations(
    citations: &[Citation],
    cl: &CourtListenerClient,
) -> Vec<VerificationResult> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut results = Vec::with_capacity(citations.len());

    for citation in citations {
        let result = match citation.citation_type.as_str() {
            "case" => verify_case_citation(citation, cl, &now).await,
            _ => VerificationResult {
                citation_text: citation.text.clone(),
                citation_type: citation.citation_type.clone(),
                status: "format_valid".into(),
                source: "format_check".into(),
                treatment: String::new(),
                checked_at: now.clone(),
            },
        };
        results.push(result);
    }

    results
}

async fn verify_case_citation(
    citation: &Citation,
    cl: &CourtListenerClient,
    now: &str,
) -> VerificationResult {
    let cite_query = if !citation.volume.is_empty() && !citation.reporter.is_empty() && !citation.page.is_empty() {
        format!("{} {} {}", citation.volume, citation.reporter, citation.page)
    } else {
        citation.text.clone()
    };

    match cl.citation_lookup(&cite_query).await {
        Ok(result) if result.count > 0 => VerificationResult {
            citation_text: citation.text.clone(),
            citation_type: citation.citation_type.clone(),
            status: "verified".into(),
            source: "courtlistener".into(),
            treatment: format!("{} match(es) found", result.count),
            checked_at: now.to_string(),
        },
        Ok(_) => VerificationResult {
            citation_text: citation.text.clone(),
            citation_type: citation.citation_type.clone(),
            status: "unverified".into(),
            source: "courtlistener".into(),
            treatment: "No matches found in CourtListener database".into(),
            checked_at: now.to_string(),
        },
        Err(e) => VerificationResult {
            citation_text: citation.text.clone(),
            citation_type: citation.citation_type.clone(),
            status: "error".into(),
            source: "courtlistener".into(),
            treatment: format!("Lookup failed: {e}"),
            checked_at: now.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_us_cases() {
        let text = "The court held in 550 U.S. 124 that something. See also 123 F.3d 456 (9th Cir. 1999).";
        let cites = extract_citations(text);
        assert!(cites.len() >= 2);
        assert!(cites.iter().any(|c| c.text.contains("550 U.S. 124")));
        assert!(cites.iter().any(|c| c.text.contains("123 F.3d 456")));
    }

    #[test]
    fn test_extract_statutes() {
        let text = "Under 42 U.S.C. § 1983 and 29 C.F.R. § 541.100, the claim is valid.";
        let cites = extract_citations(text);
        assert!(cites.iter().any(|c| c.citation_type == "statute"));
        assert!(cites.iter().any(|c| c.citation_type == "regulation"));
    }

    #[test]
    fn test_extract_uk_citations() {
        let text = "The Supreme Court ruled in [2021] UKSC 35 that...";
        let cites = extract_citations(text);
        assert!(cites.iter().any(|c| c.text.contains("[2021] UKSC 35")));
    }

    #[test]
    fn test_extract_canadian() {
        let text = "In 2023 SCC 12, the Court held that...";
        let cites = extract_citations(text);
        assert!(cites.iter().any(|c| c.text.contains("2023 SCC 12")));
    }

    #[test]
    fn test_deduplication() {
        let text = "550 U.S. 124 was cited multiple times. See 550 U.S. 124 again.";
        let cites = extract_citations(text);
        assert_eq!(cites.iter().filter(|c| c.text == "550 U.S. 124").count(), 1);
    }

    #[test]
    fn test_federal_reporters_fsupp2d_and_fappx() {
        let text = "See 123 F.Supp.2d 456 (S.D.N.Y. 2002) and 78 F.App'x 910 (2d Cir. 2003).";
        let cites = extract_citations(text);
        assert!(cites.iter().any(|c| c.text.contains("123 F.Supp.2d 456")));
        assert!(cites.iter().any(|c| c.text.contains("78 F.App'x 910")));
    }

    #[test]
    fn test_state_regional_reporters() {
        let text = "See 200 N.E.2d 300 and 400 P.3d 500.";
        let cites = extract_citations(text);
        assert!(cites.iter().any(|c| c.text.contains("200 N.E.2d 300")));
        assert!(cites.iter().any(|c| c.text.contains("400 P.3d 500")));
    }

    #[test]
    fn test_state_specific_reporters() {
        let text = "Held in 10 Cal. 20 and reversed in 30 N.Y. 40.";
        let cites = extract_citations(text);
        assert!(cites.iter().any(|c| c.text.contains("10 Cal. 20")));
        assert!(cites.iter().any(|c| c.text.contains("30 N.Y. 40")));
    }

    #[test]
    fn test_eu_court_citations() {
        let text = "The CJEU decided Case C-1/23 and the General Court decided Case T-1/23.";
        let cites = extract_citations(text);
        assert!(cites.iter().any(|c| c.text.contains("Case C-1/23")));
        assert!(cites.iter().any(|c| c.text.contains("Case T-1/23")));
    }

    #[test]
    fn test_canlii_neutral_citation() {
        let text = "As decided in 2022 CanLII 98765 (ON).";
        let cites = extract_citations(text);
        assert!(cites.iter().any(|c| c.text.contains("2022 CanLII 98765 (ON)")));
    }

    #[test]
    fn test_state_statute() {
        let text = "Pursuant to Cal. Civ. Code § 1714 and Tex. Bus. & Com. Code § 17.50.";
        let cites = extract_citations(text);
        assert!(cites.iter().any(|c| c.citation_type == "statute" && c.text.contains("Cal.")));
        assert!(cites.iter().any(|c| c.citation_type == "statute" && c.text.contains("Tex.")));
    }

    #[test]
    fn test_empty_string_returns_empty() {
        let cites = extract_citations("");
        assert!(cites.is_empty());
    }
}
