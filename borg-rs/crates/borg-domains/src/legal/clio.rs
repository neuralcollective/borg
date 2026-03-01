use anyhow::Result;
use serde::{Deserialize, Serialize};

pub struct ClioClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeEntry {
    pub matter_id: u64,
    pub date: String,
    pub quantity: f64,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_code: Option<String>,
}

impl ClioClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            base_url: "https://app.clio.com/api/v4".into(),
            api_key: api_key.to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub async fn search_matters(&self, query: &str) -> Result<serde_json::Value> {
        Ok(self.http.get(format!("{}/matters?query={}", self.base_url, urlencoding::encode(query)))
            .bearer_auth(&self.api_key).send().await?
            .error_for_status()?.json().await?)
    }

    pub async fn get_matter(&self, id: u64) -> Result<serde_json::Value> {
        Ok(self.http.get(format!("{}/matters/{id}", self.base_url))
            .bearer_auth(&self.api_key).send().await?
            .error_for_status()?.json().await?)
    }

    pub async fn create_time_entry(&self, entry: &TimeEntry) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "data": {
                "matter": { "id": entry.matter_id },
                "date": entry.date,
                "quantity": entry.quantity,
                "note": entry.description,
            }
        });
        Ok(self.http.post(format!("{}/activities", self.base_url))
            .bearer_auth(&self.api_key).json(&body).send().await?
            .error_for_status()?.json().await?)
    }

    pub async fn search_contacts(&self, query: &str) -> Result<serde_json::Value> {
        Ok(self.http.get(format!("{}/contacts?query={}", self.base_url, urlencoding::encode(query)))
            .bearer_auth(&self.api_key).send().await?
            .error_for_status()?.json().await?)
    }

    pub async fn search_documents(&self, query: &str) -> Result<serde_json::Value> {
        Ok(self.http.get(format!("{}/documents?query={}", self.base_url, urlencoding::encode(query)))
            .bearer_auth(&self.api_key).send().await?
            .error_for_status()?.json().await?)
    }
}
