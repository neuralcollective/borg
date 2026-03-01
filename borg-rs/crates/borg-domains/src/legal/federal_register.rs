use anyhow::Result;

const BASE: &str = "https://www.federalregister.gov/api/v1";

pub struct FederalRegisterClient {
    http: reqwest::Client,
}

impl FederalRegisterClient {
    pub fn new() -> Self {
        Self { http: reqwest::Client::new() }
    }

    pub async fn search(&self, term: &str) -> Result<serde_json::Value> {
        let url = format!("{BASE}/documents?conditions[term]={}", urlencoding::encode(term));
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }

    pub async fn get_document(&self, document_number: &str) -> Result<serde_json::Value> {
        let url = format!("{BASE}/documents/{document_number}");
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }

    pub async fn get_agency(&self, slug: &str) -> Result<serde_json::Value> {
        let url = format!("{BASE}/agencies/{slug}");
        Ok(self.http.get(&url).send().await?.error_for_status()?.json().await?)
    }
}
