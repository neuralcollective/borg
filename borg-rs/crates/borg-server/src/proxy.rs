use std::sync::Arc;

use axum::{
    body::Body,
    extract::{State, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use aws_config::BehaviorVersion;
use aws_sdk_bedrockruntime::Client as BedrockClient;
use aws_sdk_bedrockruntime::primitives::Blob;
use tracing::error;

#[derive(Clone)]
pub struct ProxyState {
    pub bedrock: BedrockClient,
    pub db: Arc<borg_core::db::Db>,
    pub region: String,
}

impl ProxyState {
    pub async fn new(db: Arc<borg_core::db::Db>) -> Self {
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let bedrock = BedrockClient::new(&config);
        Self {
            bedrock,
            db,
            region,
        }
    }
}

pub fn proxy_routes() -> Router<Arc<ProxyState>> {
    Router::new()
        .route("/v1/messages", post(handle_anthropic_messages))
        .route("/v1/search", post(handle_web_search))
}

#[derive(serde::Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub project_id: i64,
}

async fn handle_web_search(
    State(state): State<Arc<ProxyState>>,
    axum::Json(payload): axum::Json<SearchRequest>,
) -> Result<axum::Json<serde_json::Value>, StatusCode> {
    // 1. Check if session is privileged
    let is_privileged = state.db.is_session_privileged(payload.project_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    if is_privileged {
        error!("search blocked: session is privileged for project {}", payload.project_id);
        return Err(StatusCode::FORBIDDEN);
    }

    // 2. Perform search
    let key = state.db.get_api_key("global", "brave_search")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::PRECONDITION_FAILED)?;

    let client = borg_core::knowledge::BraveSearchClient::new(key);
    let results = client.search(&payload.query).await.map_err(|e| {
        error!("brave search failed: {e}");
        StatusCode::BAD_GATEWAY
    })?;

    Ok(axum::Json(serde_json::json!({ "results": results })))
}

async fn handle_anthropic_messages(
    State(state): State<Arc<ProxyState>>,
    req: Request,
) -> Result<Response, StatusCode> {
    let body_bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut json_body: serde_json::Value = serde_json::from_slice(&body_bytes)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    
    let anthropic_model = json_body.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let bedrock_model = map_model_id(anthropic_model);
    let is_streaming = json_body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    
    if let Some(obj) = json_body.as_object_mut() {
        obj.remove("model");
        if !obj.contains_key("anthropic_version") {
            obj.insert("anthropic_version".to_string(), "bedrock-2023-05-31".into());
        }
    }
    let new_body_bytes = serde_json::to_vec(&json_body).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if is_streaming {
        let mut output = state.bedrock.invoke_model_with_response_stream()
            .model_id(bedrock_model)
            .body(Blob::new(new_body_bytes))
            .send()
            .await
            .map_err(|e| {
                error!("bedrock stream request failed: {e}");
                StatusCode::BAD_GATEWAY
            })?;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            while let Ok(Some(event)) = output.body.recv().await {
                if let Ok(chunk) = event.as_chunk() {
                    if let Some(blob) = chunk.bytes() {
                        let bytes = blob.as_ref();
                        let json_str = String::from_utf8_lossy(bytes);
                        let sse_line = format!("data: {}\n\n", json_str);
                        if tx.send(Ok::<_, anyhow::Error>(sse_line)).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
        Ok(Body::from_stream(stream).into_response())
    } else {
        let output = state.bedrock.invoke_model()
            .model_id(bedrock_model)
            .body(Blob::new(new_body_bytes))
            .send()
            .await
            .map_err(|e| {
                error!("bedrock request failed: {e}");
                StatusCode::BAD_GATEWAY
            })?;

        let body = output.body.into_inner();
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap())
    }
}

fn map_model_id(anthropic_id: &str) -> &'static str {
    if anthropic_id.contains("opus") {
        "anthropic.claude-3-opus-20240229-v1:0"
    } else if anthropic_id.contains("sonnet-3-5") || anthropic_id.contains("sonnet-20240620") {
        "anthropic.claude-3-5-sonnet-20240620-v1:0"
    } else if anthropic_id.contains("sonnet-4-6") {
        "anthropic.claude-4-6-sonnet-20260217-v1:0"
    } else {
        "anthropic.claude-3-5-sonnet-20240620-v1:0"
    }
}
