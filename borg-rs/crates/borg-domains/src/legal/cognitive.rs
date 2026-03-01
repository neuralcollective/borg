use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Client for the LexisNexis Cognitive APIs.
/// Provides judge/court entity resolution, legal dictionary, translation, and PII redaction.
pub struct CognitiveClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityResult {
    pub canonical_name: String,
    pub entity_id: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionaryEntry {
    pub term: String,
    pub definition: String,
    pub context: Option<String>,
    pub related_terms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionResult {
    pub redacted_text: String,
    pub entities_found: Vec<RedactedEntity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedEntity {
    pub entity_type: String,
    pub original: String,
    pub start: usize,
    pub end: usize,
}

impl CognitiveClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            base_url: "https://api.lexisnexis.com/cognitive/v1".into(),
            api_key: api_key.to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    /// Resolve a judge name to a canonical entity with metadata.
    pub async fn resolve_judge(&self, name: &str) -> Result<Vec<EntityResult>> {
        let resp = self
            .http
            .get(format!("{}/entities/judges", self.base_url))
            .bearer_auth(&self.api_key)
            .query(&[("name", name)])
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Resolve a court name/abbreviation to a canonical entity.
    pub async fn resolve_court(&self, name: &str) -> Result<Vec<EntityResult>> {
        let resp = self
            .http
            .get(format!("{}/entities/courts", self.base_url))
            .bearer_auth(&self.api_key)
            .query(&[("name", name)])
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Look up a legal term in the dictionary.
    pub async fn define(&self, term: &str) -> Result<DictionaryEntry> {
        let resp = self
            .http
            .get(format!("{}/dictionary/{}", self.base_url, term))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Translate legal text to a target language.
    pub async fn translate(&self, text: &str, target_language: &str) -> Result<String> {
        let body = serde_json::json!({
            "text": text,
            "target_language": target_language,
        });
        let resp: serde_json::Value = self
            .http
            .post(format!("{}/translate", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp["translated_text"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }

    /// Detect and redact PII from text.
    pub async fn redact_pii(&self, text: &str) -> Result<RedactionResult> {
        let body = serde_json::json!({ "text": text });
        let resp = self
            .http
            .post(format!("{}/redact", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }
}
