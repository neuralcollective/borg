use std::sync::Arc;

use aws_config::BehaviorVersion;
use aws_sdk_bedrockruntime::{primitives::Blob, Client as BedrockClient};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use tracing::{error, warn};

#[derive(Clone)]
pub struct ProxyAuditLogger {
    cloudwatch: Option<CloudWatchAuditSink>,
}

#[derive(Clone)]
struct CloudWatchAuditSink {
    client: aws_sdk_cloudwatchlogs::Client,
    group: String,
    stream: String,
}

impl ProxyAuditLogger {
    async fn from_env() -> Self {
        let log_group = std::env::var("PROXY_AUDIT_CLOUDWATCH_LOG_GROUP")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let Some(group) = log_group else {
            return Self { cloudwatch: None };
        };

        let region = std::env::var("PROXY_AUDIT_CLOUDWATCH_REGION")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| std::env::var("AWS_REGION").ok())
            .unwrap_or_else(|| "us-east-1".to_string());
        let stream = std::env::var("PROXY_AUDIT_CLOUDWATCH_STREAM")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| {
                format!(
                    "proxy-{}-{}",
                    std::process::id(),
                    chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
                )
            });

        let shared = aws_config::defaults(BehaviorVersion::latest())
            .region(aws_config::Region::new(region))
            .load()
            .await;
        let client = aws_sdk_cloudwatchlogs::Client::new(&shared);

        if let Err(e) = client
            .create_log_group()
            .log_group_name(&group)
            .send()
            .await
        {
            let msg = e.to_string();
            if !msg.contains("ResourceAlreadyExistsException") {
                warn!("cloudwatch create_log_group failed: {msg}");
            }
        }
        if let Err(e) = client
            .create_log_stream()
            .log_group_name(&group)
            .log_stream_name(&stream)
            .send()
            .await
        {
            let msg = e.to_string();
            if !msg.contains("ResourceAlreadyExistsException") {
                warn!("cloudwatch create_log_stream failed: {msg}");
            }
        }

        Self {
            cloudwatch: Some(CloudWatchAuditSink {
                client,
                group,
                stream,
            }),
        }
    }

    async fn log_proxy_call(
        &self,
        call: &str,
        actor_id: &str,
        project_id: Option<i64>,
        status: &str,
        detail: serde_json::Value,
    ) {
        let payload = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "call": call,
            "actor_id": actor_id,
            "project_id": project_id,
            "status": status,
            "detail": detail,
        });

        tracing::info!(target: "proxy.audit", "{}", payload);

        let Some(cw) = &self.cloudwatch else {
            return;
        };
        let event = aws_sdk_cloudwatchlogs::types::InputLogEvent::builder()
            .message(payload.to_string())
            .timestamp(chrono::Utc::now().timestamp_millis())
            .build();
        let Ok(event) = event else {
            warn!("cloudwatch input log event build failed");
            return;
        };
        if let Err(e) = cw
            .client
            .put_log_events()
            .log_group_name(&cw.group)
            .log_stream_name(&cw.stream)
            .log_events(event)
            .send()
            .await
        {
            warn!("cloudwatch put_log_events failed: {e}");
        }
    }
}

#[derive(Clone)]
pub struct ProxyState {
    pub bedrock: BedrockClient,
    pub db: Arc<borg_core::db::Db>,
    pub audit: ProxyAuditLogger,
}

impl ProxyState {
    pub async fn new(db: Arc<borg_core::db::Db>) -> Self {
        let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let bedrock = BedrockClient::new(&config);
        let audit = ProxyAuditLogger::from_env().await;
        Self { bedrock, db, audit }
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
    #[serde(default)]
    pub actor_id: Option<String>,
}

async fn handle_web_search(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    axum::Json(payload): axum::Json<SearchRequest>,
) -> Result<axum::Json<serde_json::Value>, StatusCode> {
    let actor_id = payload
        .actor_id
        .filter(|a| !a.trim().is_empty())
        .or_else(|| actor_id_from_headers(&headers))
        .unwrap_or_else(|| "unknown".to_string());

    // 1. Check if session is privileged
    let is_privileged = state
        .db
        .is_session_privileged(payload.project_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if is_privileged {
        error!(
            "search blocked: session is privileged for project {}",
            payload.project_id
        );
        state
            .audit
            .log_proxy_call(
                "web_search",
                &actor_id,
                Some(payload.project_id),
                "forbidden",
                serde_json::json!({ "reason": "session_privileged", "query_len": payload.query.len() }),
            )
            .await;
        return Err(StatusCode::FORBIDDEN);
    }

    // 2. Perform search
    let key = state
        .db
        .get_api_key("global", "brave_search")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::PRECONDITION_FAILED)?;

    let client = borg_core::knowledge::BraveSearchClient::new(key);
    let results = client.search(&payload.query).await.map_err(|e| {
        error!("brave search failed: {e}");
        StatusCode::BAD_GATEWAY
    })?;
    state
        .audit
        .log_proxy_call(
            "web_search",
            &actor_id,
            Some(payload.project_id),
            "ok",
            serde_json::json!({ "result_count": results.len(), "query_len": payload.query.len() }),
        )
        .await;

    Ok(axum::Json(serde_json::json!({ "results": results })))
}

async fn handle_anthropic_messages(
    State(state): State<Arc<ProxyState>>,
    req: Request,
) -> Result<Response, StatusCode> {
    let headers = req.headers().clone();
    let body_bytes = axum::body::to_bytes(req.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let mut json_body: serde_json::Value =
        serde_json::from_slice(&body_bytes).map_err(|_| StatusCode::BAD_REQUEST)?;

    let actor_id = actor_id_from_headers(&headers).unwrap_or_else(|| "unknown".to_string());
    let project_id = project_id_from_headers(&headers)
        .or_else(|| json_body.get("project_id").and_then(|v| v.as_i64()));
    let anthropic_model = json_body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let bedrock_model = map_model_id(&anthropic_model);
    let is_streaming = json_body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if let Some(obj) = json_body.as_object_mut() {
        obj.remove("model");
        if !obj.contains_key("anthropic_version") {
            obj.insert("anthropic_version".to_string(), "bedrock-2023-05-31".into());
        }
    }
    let new_body_bytes =
        serde_json::to_vec(&json_body).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if is_streaming {
        let mut output = state
            .bedrock
            .invoke_model_with_response_stream()
            .model_id(bedrock_model)
            .body(Blob::new(new_body_bytes))
            .send()
            .await
            .map_err(|e| {
                error!("bedrock stream request failed: {e}");
                StatusCode::BAD_GATEWAY
            })?;
        state
            .audit
            .log_proxy_call(
                "bedrock_messages",
                &actor_id,
                project_id,
                "stream_started",
                serde_json::json!({ "anthropic_model": anthropic_model, "bedrock_model": bedrock_model }),
            )
            .await;

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
        let output = state
            .bedrock
            .invoke_model()
            .model_id(bedrock_model)
            .body(Blob::new(new_body_bytes))
            .send()
            .await
            .map_err(|e| {
                error!("bedrock request failed: {e}");
                StatusCode::BAD_GATEWAY
            })?;
        state
            .audit
            .log_proxy_call(
                "bedrock_messages",
                &actor_id,
                project_id,
                "ok",
                serde_json::json!({ "anthropic_model": anthropic_model, "bedrock_model": bedrock_model }),
            )
            .await;

        let body = output.body.into_inner();
        Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap())
    }
}

fn actor_id_from_headers(headers: &HeaderMap) -> Option<String> {
    for key in ["x-borg-actor-id", "x-actor-id", "x-user-id"] {
        if let Some(v) = headers.get(key).and_then(|h| h.to_str().ok()) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn project_id_from_headers(headers: &HeaderMap) -> Option<i64> {
    headers
        .get("x-borg-project-id")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
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
