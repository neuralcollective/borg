use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Client for the Lex Machina API.
/// Provides litigation analytics: case resolutions, damages, remedies across courts.
pub struct LexMachinaClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseSearchParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attorney: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub court: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_type: Option<String>,
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
pub struct SearchResult {
    pub total: u64,
    pub cases: Vec<serde_json::Value>,
}

impl LexMachinaClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            base_url: "https://api.lexmachina.com/v1".into(),
            api_key: api_key.to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    /// Search cases by party, attorney, judge, court, or case type.
    pub async fn search_cases(&self, params: &CaseSearchParams) -> Result<SearchResult> {
        let resp = self
            .http
            .post(format!("{}/cases/search", self.base_url))
            .bearer_auth(&self.api_key)
            .json(params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Get full case analytics: resolutions, damages, remedies, timing.
    pub async fn get_case(&self, case_id: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/cases/{}", self.base_url, case_id))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Get judge profile: ruling patterns, case duration, outcomes.
    pub async fn get_judge(&self, judge_id: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/judges/{}", self.base_url, judge_id))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Get party litigation history and analytics.
    pub async fn get_party(&self, party_name: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/parties", self.base_url))
            .bearer_auth(&self.api_key)
            .query(&[("name", party_name)])
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Get attorney analytics: experience, outcomes by practice area.
    pub async fn get_attorney(&self, attorney_id: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/attorneys/{}", self.base_url, attorney_id))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }
}
