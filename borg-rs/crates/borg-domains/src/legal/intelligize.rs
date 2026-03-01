use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Client for the Intelligize API.
/// Provides access to SEC filings, compliance data, and disclosure analysis.
pub struct IntelligizeClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilingSearchParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filing_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClauseSearchParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filing_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub total: u64,
    pub results: Vec<serde_json::Value>,
}

impl IntelligizeClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            base_url: "https://api.intelligize.com/v1".into(),
            api_key: api_key.to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    /// Search SEC filings by company, type, and date range.
    pub async fn search_filings(&self, params: &FilingSearchParams) -> Result<SearchResult> {
        let resp = self
            .http
            .post(format!("{}/filings/search", self.base_url))
            .bearer_auth(&self.api_key)
            .json(params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Retrieve a filing by ID, optionally a specific section.
    pub async fn get_filing(
        &self,
        filing_id: &str,
        section: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut url = format!("{}/filings/{}", self.base_url, filing_id);
        if let Some(s) = section {
            url.push_str(&format!("?section={}", s));
        }
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Search for specific clause language across filings.
    pub async fn search_clauses(&self, params: &ClauseSearchParams) -> Result<SearchResult> {
        let resp = self
            .http
            .post(format!("{}/clauses/search", self.base_url))
            .bearer_auth(&self.api_key)
            .json(params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Compare disclosure language across companies.
    pub async fn compare_clauses(
        &self,
        clause_query: &str,
        companies: &[String],
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "query": clause_query,
            "companies": companies,
        });
        let resp = self
            .http
            .post(format!("{}/clauses/compare", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Set up monitoring for new filings matching criteria.
    pub async fn create_monitor(
        &self,
        company: &str,
        filing_types: &[String],
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "company": company,
            "filing_types": filing_types,
        });
        let resp = self
            .http
            .post(format!("{}/monitors", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }
}
