use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Client for the LexisNexis Lexis API.
/// Provides access to case law, secondary sources, Shepard's citations, search, and alerts.
pub struct LexisClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub total: u64,
    pub results: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShepardsResult {
    pub citation: String,
    pub treatment: String,
    pub citing_references: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
}

impl LexisClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            base_url: "https://api.lexisnexis.com/v1".into(),
            api_key: api_key.to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    /// Search case law, secondary sources, and other legal content.
    pub async fn search(&self, params: &SearchParams) -> Result<SearchResult> {
        let resp = self
            .http
            .post(format!("{}/search", self.base_url))
            .bearer_auth(&self.api_key)
            .json(params)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Retrieve a full document by ID.
    pub async fn get_document(&self, document_id: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/documents/{}", self.base_url, document_id))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Check Shepard's citation treatment.
    pub async fn shepards(&self, citation: &str) -> Result<ShepardsResult> {
        let resp = self
            .http
            .get(format!("{}/shepards/{}", self.base_url, citation))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Create an alert for new content matching a query.
    pub async fn create_alert(&self, config: &AlertConfig) -> Result<serde_json::Value> {
        let resp = self
            .http
            .post(format!("{}/alerts", self.base_url))
            .bearer_auth(&self.api_key)
            .json(config)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// List active alerts.
    pub async fn list_alerts(&self) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .http
            .get(format!("{}/alerts", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }
}
