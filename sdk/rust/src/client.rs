use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::types::*;
use crate::BorgError;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:3131";
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 2_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 20 * 60 * 1000;
pub const UPLOAD_CHUNK_SIZE: usize = 256 * 1024;

pub struct BorgClientConfig {
    pub base_url: Option<String>,
    pub token: Option<String>,
    pub token_file: Option<String>,
    pub token_search_paths: Vec<String>,
}

impl Default for BorgClientConfig {
    fn default() -> Self {
        Self {
            base_url: None,
            token: None,
            token_file: None,
            token_search_paths: Vec::new(),
        }
    }
}

pub struct BorgClient {
    base_url: String,
    token: String,
    http: Client,
}

impl BorgClient {
    pub async fn new(config: BorgClientConfig) -> Result<Self, BorgError> {
        let base_url = config
            .base_url
            .clone()
            .or_else(|| std::env::var("BORG_BASE_URL").ok())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();

        let http = Client::new();
        let token = Self::resolve_token(&http, &base_url, &config).await?;
        Ok(Self { base_url, token, http })
    }

    async fn resolve_token(
        http: &Client,
        base_url: &str,
        config: &BorgClientConfig,
    ) -> Result<String, BorgError> {
        let env_candidates = [
            ("config.token", config.token.clone()),
            ("BORG_API_TOKEN", std::env::var("BORG_API_TOKEN").ok()),
            ("API_TOKEN", std::env::var("API_TOKEN").ok()),
        ];

        for (source, maybe_token) in &env_candidates {
            if let Some(token) = maybe_token.as_ref().map(|t| t.trim().to_string()) {
                if token.is_empty() {
                    continue;
                }
                if Self::token_works(http, base_url, &token).await {
                    return Ok(token);
                }
                return Err(BorgError::Auth(format!(
                    "Token from {source} was rejected by {base_url}/api/projects"
                )));
            }
        }

        let mut file_paths = Vec::new();
        if let Some(tf) = &config.token_file {
            file_paths.push(tf.clone());
        }
        file_paths.extend(config.token_search_paths.iter().cloned());

        for path in &file_paths {
            let p = Path::new(path);
            if !p.exists() {
                continue;
            }
            if let Ok(contents) = tokio::fs::read_to_string(p).await {
                let token = contents.trim().to_string();
                if token.is_empty() {
                    continue;
                }
                if Self::token_works(http, base_url, &token).await {
                    return Ok(token);
                }
            }
        }

        let url = format!("{base_url}/api/auth/token");
        if let Ok(resp) = http.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    if let Some(token) = data.get("token").and_then(|t| t.as_str()) {
                        let token = token.trim().to_string();
                        if !token.is_empty() && Self::token_works(http, base_url, &token).await {
                            return Ok(token);
                        }
                    }
                }
            }
        }

        Err(BorgError::Auth(format!(
            "Could not authenticate with Borg at {base_url}: no valid token found"
        )))
    }

    async fn token_works(http: &Client, base_url: &str, token: &str) -> bool {
        http.get(format!("{base_url}/api/projects"))
            .bearer_auth(token)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    // -- low-level helpers --

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, BorgError> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| BorgError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BorgError::Api {
                method: "GET".into(),
                path: path.into(),
                status,
                body,
            });
        }
        resp.json().await.map_err(|e| BorgError::Request(e.to_string()))
    }

    async fn get_text(&self, path: &str) -> Result<String, BorgError> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| BorgError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BorgError::Api {
                method: "GET".into(),
                path: path.into(),
                status,
                body,
            });
        }
        resp.text().await.map_err(|e| BorgError::Request(e.to_string()))
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, BorgError> {
        let resp = self
            .http
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| BorgError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BorgError::Api {
                method: "POST".into(),
                path: path.into(),
                status,
                body,
            });
        }

        let text = resp.text().await.map_err(|e| BorgError::Request(e.to_string()))?;
        if text.trim().is_empty() {
            return serde_json::from_str("null").map_err(|e| BorgError::Request(e.to_string()));
        }
        serde_json::from_str(&text).map_err(|e| BorgError::Request(e.to_string()))
    }

    async fn post_empty(&self, path: &str) -> Result<(), BorgError> {
        let resp = self
            .http
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| BorgError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BorgError::Api {
                method: "POST".into(),
                path: path.into(),
                status,
                body,
            });
        }
        Ok(())
    }

    async fn put_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, BorgError> {
        let resp = self
            .http
            .put(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| BorgError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BorgError::Api {
                method: "PUT".into(),
                path: path.into(),
                status,
                body,
            });
        }

        let text = resp.text().await.map_err(|e| BorgError::Request(e.to_string()))?;
        if text.trim().is_empty() {
            return serde_json::from_str("null").map_err(|e| BorgError::Request(e.to_string()));
        }
        serde_json::from_str(&text).map_err(|e| BorgError::Request(e.to_string()))
    }

    async fn put_bytes(&self, path: &str, bytes: &[u8]) -> Result<(), BorgError> {
        let resp = self
            .http
            .put(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .header("content-type", "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|e| BorgError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BorgError::Api {
                method: "PUT".into(),
                path: path.into(),
                status,
                body,
            });
        }
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<(), BorgError> {
        let resp = self
            .http
            .delete(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| BorgError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BorgError::Api {
                method: "DELETE".into(),
                path: path.into(),
                status,
                body,
            });
        }
        Ok(())
    }

    async fn patch<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, BorgError> {
        let resp = self
            .http
            .patch(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| BorgError::Request(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BorgError::Api {
                method: "PATCH".into(),
                path: path.into(),
                status,
                body,
            });
        }
        resp.json().await.map_err(|e| BorgError::Request(e.to_string()))
    }

    // -- Projects --

    pub async fn list_projects(&self) -> Result<Vec<Project>, BorgError> {
        self.get("/api/projects").await
    }

    pub async fn create_project(&self, body: &CreateProjectBody) -> Result<Project, BorgError> {
        self.post("/api/projects", body).await
    }

    pub async fn get_project(&self, project_id: i64) -> Result<Project, BorgError> {
        self.get(&format!("/api/projects/{project_id}")).await
    }

    pub async fn update_project(
        &self,
        project_id: i64,
        body: &UpdateProjectBody,
    ) -> Result<Project, BorgError> {
        self.put_json(&format!("/api/projects/{project_id}"), body).await
    }

    pub async fn delete_project(&self, project_id: i64) -> Result<(), BorgError> {
        self.delete(&format!("/api/projects/{project_id}")).await
    }

    pub async fn search_projects(&self, query: &str) -> Result<Vec<Project>, BorgError> {
        let encoded = urlencoding::encode(query);
        self.get(&format!("/api/projects/search?q={encoded}")).await
    }

    // -- Tasks --

    pub async fn list_tasks(&self, repo: Option<&str>) -> Result<Vec<Task>, BorgError> {
        match repo {
            Some(r) => {
                let encoded = urlencoding::encode(r);
                self.get(&format!("/api/tasks?repo={encoded}")).await
            }
            None => self.get("/api/tasks").await,
        }
    }

    pub async fn create_task(&self, body: &CreateTaskBody) -> Result<Task, BorgError> {
        self.post("/api/tasks/create", body).await
    }

    pub async fn get_task(&self, task_id: i64) -> Result<Task, BorgError> {
        self.get(&format!("/api/tasks/{task_id}")).await
    }

    pub async fn patch_task(
        &self,
        task_id: i64,
        body: &PatchTaskBody,
    ) -> Result<Task, BorgError> {
        self.patch(&format!("/api/tasks/{task_id}"), body).await
    }

    pub async fn get_task_outputs(
        &self,
        task_id: i64,
    ) -> Result<TaskOutputsResponse, BorgError> {
        self.get(&format!("/api/tasks/{task_id}/outputs")).await
    }

    pub async fn get_task_messages(
        &self,
        task_id: i64,
    ) -> Result<TaskMessagesResponse, BorgError> {
        self.get(&format!("/api/tasks/{task_id}/messages")).await
    }

    pub async fn post_task_message(
        &self,
        task_id: i64,
        role: &str,
        content: &str,
    ) -> Result<TaskMessage, BorgError> {
        #[derive(Serialize)]
        struct Body<'a> {
            role: &'a str,
            content: &'a str,
        }
        self.post(
            &format!("/api/tasks/{task_id}/messages"),
            &Body { role, content },
        )
        .await
    }

    pub async fn approve_task(&self, task_id: i64) -> Result<(), BorgError> {
        self.post_empty(&format!("/api/tasks/{task_id}/approve")).await
    }

    pub async fn reject_task(&self, task_id: i64) -> Result<(), BorgError> {
        self.post_empty(&format!("/api/tasks/{task_id}/reject")).await
    }

    pub async fn retry_task(&self, task_id: i64) -> Result<(), BorgError> {
        self.post_empty(&format!("/api/tasks/{task_id}/retry")).await
    }

    pub async fn unblock_task(&self, task_id: i64, response: &str) -> Result<(), BorgError> {
        #[derive(Serialize)]
        struct Body<'a> {
            response: &'a str,
        }
        let _: serde_json::Value = self
            .post(&format!("/api/tasks/{task_id}/unblock"), &Body { response })
            .await?;
        Ok(())
    }

    pub async fn request_task_revision(
        &self,
        task_id: i64,
        feedback: &str,
    ) -> Result<RevisionResponse, BorgError> {
        #[derive(Serialize)]
        struct Body<'a> {
            feedback: &'a str,
        }
        self.post(
            &format!("/api/tasks/{task_id}/request-revision"),
            &Body { feedback },
        )
        .await
    }

    pub async fn set_task_backend(
        &self,
        task_id: i64,
        backend: &str,
    ) -> Result<TaskBackendResponse, BorgError> {
        #[derive(Serialize)]
        struct Body<'a> {
            backend: &'a str,
        }
        self.put_json(&format!("/api/tasks/{task_id}/backend"), &Body { backend })
            .await
    }

    pub async fn get_project_tasks(&self, project_id: i64) -> Result<Vec<Task>, BorgError> {
        self.get(&format!("/api/projects/{project_id}/tasks")).await
    }

    // -- Files & Uploads --

    pub async fn list_project_files(
        &self,
        project_id: i64,
        limit: i64,
    ) -> Result<ProjectFilesResponse, BorgError> {
        self.get(&format!("/api/projects/{project_id}/files?limit={limit}"))
            .await
    }

    pub async fn get_project_file(
        &self,
        project_id: i64,
        file_id: i64,
    ) -> Result<ProjectFile, BorgError> {
        self.get(&format!("/api/projects/{project_id}/files/{file_id}"))
            .await
    }

    pub async fn delete_project_file(
        &self,
        project_id: i64,
        file_id: i64,
    ) -> Result<(), BorgError> {
        self.delete(&format!("/api/projects/{project_id}/files/{file_id}"))
            .await
    }

    pub async fn delete_all_project_files(&self, project_id: i64) -> Result<(), BorgError> {
        self.delete(&format!("/api/projects/{project_id}/files"))
            .await
    }

    pub async fn create_upload_session(
        &self,
        project_id: i64,
        body: &CreateUploadSessionBody,
    ) -> Result<UploadSession, BorgError> {
        self.post(
            &format!("/api/projects/{project_id}/uploads/sessions"),
            body,
        )
        .await
    }

    pub async fn upload_chunk(
        &self,
        project_id: i64,
        session_id: i64,
        chunk_index: i64,
        bytes: &[u8],
    ) -> Result<(), BorgError> {
        self.put_bytes(
            &format!("/api/projects/{project_id}/uploads/sessions/{session_id}/chunks/{chunk_index}"),
            bytes,
        )
        .await
    }

    pub async fn complete_upload(
        &self,
        project_id: i64,
        session_id: i64,
    ) -> Result<(), BorgError> {
        self.post_empty(&format!(
            "/api/projects/{project_id}/uploads/sessions/{session_id}/complete"
        ))
        .await
    }

    // -- Documents --

    pub async fn list_project_documents(
        &self,
        project_id: i64,
    ) -> Result<Vec<ProjectDocument>, BorgError> {
        self.get(&format!("/api/projects/{project_id}/documents"))
            .await
    }

    pub async fn get_project_document_content(
        &self,
        project_id: i64,
        task_id: i64,
        path: &str,
        ref_name: Option<&str>,
    ) -> Result<String, BorgError> {
        let encoded_path = urlencoding::encode(path);
        let mut url = format!(
            "/api/projects/{project_id}/documents/{task_id}/content?path={encoded_path}"
        );
        if let Some(r) = ref_name {
            url.push_str(&format!("&ref_name={}", urlencoding::encode(r)));
        }
        self.get_text(&url).await
    }

    pub async fn delete_project_document(
        &self,
        project_id: i64,
        task_id: i64,
    ) -> Result<(), BorgError> {
        self.delete(&format!("/api/projects/{project_id}/documents/{task_id}"))
            .await
    }

    // -- Chat --

    pub async fn post_project_chat(
        &self,
        project_id: i64,
        text: &str,
        sender: Option<&str>,
    ) -> Result<(), BorgError> {
        let body = ChatPostBody {
            text: text.to_string(),
            sender: sender.map(|s| s.to_string()),
            thread: None,
            model: None,
        };
        let _: serde_json::Value = self
            .post(&format!("/api/projects/{project_id}/chat"), &body)
            .await?;
        Ok(())
    }

    pub async fn get_project_chat_messages(
        &self,
        project_id: i64,
        limit: i64,
    ) -> Result<Vec<ChatMessage>, BorgError> {
        self.get(&format!(
            "/api/projects/{project_id}/chat/messages?limit={limit}"
        ))
        .await
    }

    // -- Search --

    pub async fn search(
        &self,
        query: &str,
        project_id: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<SearchResult>, BorgError> {
        let mut params = format!("q={}", urlencoding::encode(query));
        if let Some(pid) = project_id {
            params.push_str(&format!("&project_id={pid}"));
        }
        if let Some(lim) = limit {
            params.push_str(&format!("&limit={lim}"));
        }
        self.get(&format!("/api/search?{params}")).await
    }

    // -- System --

    pub async fn get_status(&self) -> Result<SystemStatus, BorgError> {
        self.get("/api/status").await
    }

    pub async fn get_health(&self) -> bool {
        self.http
            .get(format!("{}/api/health", self.base_url))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }
}

// Extra response wrappers
#[derive(Debug, Clone, Deserialize)]
pub struct TaskOutputsResponse {
    pub outputs: Vec<TaskOutput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskMessagesResponse {
    pub messages: Vec<TaskMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RevisionResponse {
    pub ok: bool,
    pub target_phase: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskBackendResponse {
    pub ok: Option<bool>,
    pub backend: Option<String>,
}
