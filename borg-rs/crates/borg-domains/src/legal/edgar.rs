use anyhow::Result;

const EFTS: &str = "https://efts.sec.gov/LATEST";
const DATA: &str = "https://data.sec.gov";

pub struct EdgarClient {
    http: reqwest::Client,
}

impl EdgarClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("LegalMCP/0.2 (borg-legal-agent; contact@neuralcollective.ai)")
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn fulltext_search(&self, query: &str) -> Result<serde_json::Value> {
        let url = format!("{EFTS}/search-index?q={}", urlencoding::encode(query));
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }

    pub async fn company_filings(&self, cik: &str) -> Result<serde_json::Value> {
        let cik = format!("{:0>10}", cik);
        let url = format!("{DATA}/submissions/CIK{cik}.json");
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }

    pub async fn company_facts(&self, cik: &str) -> Result<serde_json::Value> {
        let cik = format!("{:0>10}", cik);
        let url = format!("{DATA}/api/xbrl/companyfacts/CIK{cik}.json");
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }

    pub async fn company_concept(&self, cik: &str, taxonomy: &str, concept: &str) -> Result<serde_json::Value> {
        let cik = format!("{:0>10}", cik);
        let url = format!("{DATA}/api/xbrl/companyconcept/CIK{cik}/{taxonomy}/{concept}.json");
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }
}
