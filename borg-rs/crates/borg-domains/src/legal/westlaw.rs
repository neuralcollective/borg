use anyhow::Result;
use serde::{Deserialize, Serialize};

pub struct WestlawClient {
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
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl WestlawClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            base_url: "https://api.thomsonreuters.com/legal/v1".into(),
            api_key: api_key.to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub async fn search(&self, params: &SearchParams) -> Result<serde_json::Value> {
        Ok(self.http.post(format!("{}/search", self.base_url))
            .bearer_auth(&self.api_key).json(params).send().await?
            .error_for_status()?.json().await?)
    }

    pub async fn get_document(&self, document_id: &str) -> Result<serde_json::Value> {
        Ok(self.http.get(format!("{}/documents/{document_id}", self.base_url))
            .bearer_auth(&self.api_key).send().await?
            .error_for_status()?.json().await?)
    }

    pub async fn keycite(&self, citation: &str) -> Result<serde_json::Value> {
        Ok(self.http.get(format!("{}/keycite/{}", self.base_url, urlencoding::encode(citation)))
            .bearer_auth(&self.api_key).send().await?
            .error_for_status()?.json().await?)
    }

    pub async fn practical_law_search(&self, query: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "query": query });
        Ok(self.http.post(format!("{}/practical-law/search", self.base_url))
            .bearer_auth(&self.api_key).json(&body).send().await?
            .error_for_status()?.json().await?)
    }

    pub async fn litigation_analytics(&self, query_type: &str, query: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "query": query });
        Ok(self.http.post(format!("{}/analytics/{query_type}", self.base_url))
            .bearer_auth(&self.api_key).json(&body).send().await?
            .error_for_status()?.json().await?)
    }
}
