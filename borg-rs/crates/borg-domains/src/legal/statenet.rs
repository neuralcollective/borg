use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Client for the LexisNexis State Net API.
/// Provides access to bills, regulations, statutes, and administrative codes.
pub struct StateNetClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillSearchParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegulationSearchParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub total: u64,
    pub results: Vec<serde_json::Value>,
}

impl StateNetClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            base_url: "https://api.lexisnexis.com/statenet/v1".into(),
            api_key: api_key.to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    /// Search for bills by keyword, state, session, and status.
    pub async fn search_bills(&self, params: &BillSearchParams) -> Result<SearchResult> {
        let resp = self
            .http
            .post(format!("{}/bills/search", self.base_url))
            .bearer_auth(&self.api_key)
            .json(params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Retrieve full bill details including text, history, and sponsors.
    pub async fn get_bill(&self, bill_id: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/bills/{}", self.base_url, bill_id))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Search federal register and state regulations.
    pub async fn search_regulations(
        &self,
        params: &RegulationSearchParams,
    ) -> Result<SearchResult> {
        let resp = self
            .http
            .post(format!("{}/regulations/search", self.base_url))
            .bearer_auth(&self.api_key)
            .json(params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Retrieve statute text by citation.
    pub async fn get_statute(&self, citation: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/statutes/{}", self.base_url, citation))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Track a bill for status changes.
    pub async fn track_bill(&self, bill_id: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .post(format!("{}/bills/{}/track", self.base_url, bill_id))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }
}
