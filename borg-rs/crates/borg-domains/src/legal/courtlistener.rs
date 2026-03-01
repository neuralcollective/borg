use anyhow::Result;
use serde::{Deserialize, Serialize};

const BASE: &str = "https://www.courtlistener.com/api/rest/v4";

pub struct CourtListenerClient {
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub count: u64,
    pub results: Vec<serde_json::Value>,
    pub next: Option<String>,
}

impl CourtListenerClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("LegalMCP/0.2 (borg-legal-agent)")
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn search_opinions(&self, query: &str, court: Option<&str>) -> Result<SearchResult> {
        let mut url = format!("{BASE}/search/?q={}&type=o", urlencoding::encode(query));
        if let Some(c) = court {
            url.push_str(&format!("&court={c}"));
        }
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }

    pub async fn get_opinion(&self, id: u64) -> Result<serde_json::Value> {
        Ok(self.http.get(format!("{BASE}/clusters/{id}/")).send().await?.error_for_status()?.json().await?)
    }

    pub async fn search_dockets(&self, query: &str) -> Result<SearchResult> {
        let url = format!("{BASE}/search/?q={}&type=d", urlencoding::encode(query));
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }

    pub async fn get_docket(&self, id: u64) -> Result<serde_json::Value> {
        Ok(self.http.get(format!("{BASE}/dockets/{id}/")).send().await?.error_for_status()?.json().await?)
    }

    pub async fn search_judges(&self, query: &str) -> Result<SearchResult> {
        let url = format!("{BASE}/search/?q={}&type=p", urlencoding::encode(query));
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }

    pub async fn citation_lookup(&self, cite: &str) -> Result<SearchResult> {
        let url = format!("{BASE}/search/?q={}&type=o", urlencoding::encode(cite));
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }
}
