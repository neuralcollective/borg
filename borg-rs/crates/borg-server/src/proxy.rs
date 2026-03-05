use std::sync::Arc;

use anyhow::Result;
use axum::{
    body::Body,
    extract::{State, Request},
    http::{StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use aws_config::BehaviorVersion;
use aws_credential_types::provider::ProvideCredentials;
use aws_sigv4::http_request::{sign, SignableRequest, SigningSettings, SignableBody, SigningParams};
use aws_sigv4::sign::v4;
use http::Uri;
use tracing::error;

#[derive(Clone)]
pub struct ProxyState {
    pub client: reqwest::Client,
    pub region: String,
}

impl ProxyState {
    pub async fn new() -> Self {
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        Self {
            client: reqwest::Client::new(),
            region,
        }
    }
}

pub fn proxy_routes() -> Router<Arc<ProxyState>> {
    Router::new()
        .route("/v1/messages", post(handle_anthropic_messages))
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
    
    if let Some(obj) = json_body.as_object_mut() {
        obj.remove("model");
        if !obj.contains_key("anthropic_version") {
            obj.insert("anthropic_version".to_string(), "bedrock-2023-05-31".into());
        }
    }
    let new_body_bytes = serde_json::to_vec(&json_body).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let url_str = format!(
        "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke-with-response-stream",
        state.region, bedrock_model
    );
    let uri: Uri = url_str.parse().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let credentials = config.credentials_provider().unwrap().provide_credentials().await
        .map_err(|e| {
            error!("failed to load aws credentials: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    
    let signing_settings = SigningSettings::default();
    
    let v4_params = v4::SigningParams {
        access_key: credentials.access_key_id(),
        secret_key: credentials.secret_access_key(),
        security_token: credentials.session_token(),
        region: &state.region,
        service_name: "bedrock",
        time: std::time::SystemTime::now(),
        settings: signing_settings,
    };
    
    let signing_params = SigningParams::V4(v4_params);

    let headers_vec: Vec<(&str, &str)> = vec![]; 
    
    let signable = SignableRequest::new(
        "POST",
        &url_str, 
        headers_vec.into_iter(),
        SignableBody::Bytes(&new_body_bytes),
    ).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let (signing_instructions, _signature) = sign(signable, &signing_params)
        .map_err(|e| {
            error!("signing failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .into_parts();

    let mut reqwest_req = state.client.post(url_str)
        .body(new_body_bytes);

    for (name, value) in signing_instructions.headers() {
        reqwest_req = reqwest_req.header(name, value);
    }

    let resp = reqwest_req.send().await.map_err(|e| {
        error!("upstream bedrock request failed: {e}");
        StatusCode::BAD_GATEWAY
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        error!("bedrock error {}: {}", status, text);
        return Err(StatusCode::BAD_GATEWAY);
    }

    Ok(Body::from_stream(resp.bytes_stream()).into_response())
}

fn map_model_id(anthropic_id: &str) -> &'static str {
    if anthropic_id.contains("opus") {
        "anthropic.claude-3-opus-20240229-v1:0"
    } else {
        "anthropic.claude-3-5-sonnet-20240620-v1:0" 
    }
}
