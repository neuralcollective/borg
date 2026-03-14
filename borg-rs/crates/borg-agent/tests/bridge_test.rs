use std::collections::HashMap;

use borg_core::traits::{
    AgentError, BackendCapabilities, ProviderConfig, RetryPolicy,
};

// ── ProviderConfig tests ─────────────────────────────────────────────────

#[test]
fn provider_subscription_produces_no_env_vars() {
    let config = ProviderConfig::Subscription;
    assert!(config.to_env_vars().is_empty());
}

#[test]
fn provider_direct_produces_api_key() {
    let config = ProviderConfig::Direct {
        api_key: "sk-test-123".into(),
    };
    let env = config.to_env_vars();
    assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test-123");
    assert_eq!(env.len(), 1);
}

#[test]
fn provider_bedrock_produces_correct_env_vars() {
    let config = ProviderConfig::Bedrock {
        region: "us-west-2".into(),
        profile: Some("production".into()),
    };
    let env = config.to_env_vars();
    assert_eq!(env.get("CLAUDE_CODE_USE_BEDROCK").unwrap(), "1");
    assert_eq!(env.get("AWS_REGION").unwrap(), "us-west-2");
    assert_eq!(env.get("AWS_PROFILE").unwrap(), "production");
    assert_eq!(env.len(), 3);
}

#[test]
fn provider_bedrock_without_profile() {
    let config = ProviderConfig::Bedrock {
        region: "eu-west-1".into(),
        profile: None,
    };
    let env = config.to_env_vars();
    assert_eq!(env.len(), 2);
    assert!(!env.contains_key("AWS_PROFILE"));
}

#[test]
fn provider_vertex_produces_correct_env_vars() {
    let config = ProviderConfig::Vertex {
        project_id: "my-project".into(),
        region: "europe-west4".into(),
    };
    let env = config.to_env_vars();
    assert_eq!(env.get("CLAUDE_CODE_USE_VERTEX").unwrap(), "1");
    assert_eq!(env.get("ANTHROPIC_VERTEX_PROJECT_ID").unwrap(), "my-project");
    assert_eq!(env.get("CLOUD_ML_REGION").unwrap(), "europe-west4");
}

// ── AgentError classification tests ──────────────────────────────────────

#[test]
fn error_rate_limit_is_retryable() {
    let err = AgentError::RateLimit {
        retry_after: Some(std::time::Duration::from_secs(30)),
    };
    assert!(err.is_retryable());
}

#[test]
fn error_server_error_is_retryable() {
    let err = AgentError::ServerError { status: 503 };
    assert!(err.is_retryable());
}

#[test]
fn error_timeout_is_retryable() {
    assert!(AgentError::Timeout.is_retryable());
}

#[test]
fn error_process_crash_is_retryable() {
    let err = AgentError::ProcessCrash {
        exit_code: Some(137),
    };
    assert!(err.is_retryable());
}

#[test]
fn error_auth_is_not_retryable() {
    assert!(!AgentError::AuthError.is_retryable());
}

#[test]
fn error_context_overflow_is_not_retryable() {
    assert!(!AgentError::ContextOverflow.is_retryable());
}

#[test]
fn error_insufficient_balance_is_not_retryable() {
    assert!(!AgentError::InsufficientBalance.is_retryable());
}

#[test]
fn error_from_code_rate_limit() {
    let err = AgentError::from_error_code("rate_limit", "too many requests");
    assert!(matches!(err, AgentError::RateLimit { .. }));
}

#[test]
fn error_from_code_auth() {
    let err = AgentError::from_error_code("auth", "invalid credentials");
    assert!(matches!(err, AgentError::AuthError));
}

#[test]
fn error_from_code_context_overflow() {
    let err = AgentError::from_error_code("context_overflow", "context too long");
    assert!(matches!(err, AgentError::ContextOverflow));
}

#[test]
fn error_from_unknown_code_classifies_by_message() {
    let err = AgentError::from_error_code("unknown", "rate limit exceeded");
    assert!(matches!(err, AgentError::RateLimit { .. }));
}

// ── RetryPolicy tests ────────────────────────────────────────────────────

#[test]
fn retry_policy_defaults_are_reasonable() {
    let policy = RetryPolicy::default();
    assert_eq!(policy.max_retries, 3);
    assert_eq!(policy.initial_backoff, std::time::Duration::from_secs(1));
    assert_eq!(policy.max_backoff, std::time::Duration::from_secs(60));
    assert_eq!(policy.backoff_multiplier, 2.0);
    assert!(policy.jitter);
}

// ── BackendCapabilities tests ────────────────────────────────────────────

#[test]
fn default_capabilities_are_all_false() {
    let caps = BackendCapabilities::default();
    assert!(!caps.supports_mcp);
    assert!(!caps.supports_sessions);
    assert!(!caps.supports_tools);
    assert!(!caps.supports_streaming);
    assert!(!caps.supports_sandbox);
    assert!(caps.supported_models.is_empty());
}

// ── AgentSdkBackend unit tests ───────────────────────────────────────────

#[test]
fn agent_sdk_backend_name() {
    let backend = borg_agent::AgentSdkBackend::new(ProviderConfig::Subscription);
    use borg_core::agent::AgentBackend;
    assert_eq!(backend.name(), "agent-sdk");
}

#[test]
fn agent_sdk_backend_capabilities() {
    let backend = borg_agent::AgentSdkBackend::new(ProviderConfig::Subscription);
    use borg_core::agent::AgentBackend;
    let caps = backend.capabilities();
    assert!(caps.supports_mcp);
    assert!(caps.supports_sessions);
    assert!(caps.supports_tools);
    assert!(caps.supports_streaming);
}

#[test]
fn claude_backend_name() {
    let backend = borg_agent::claude::ClaudeBackend::new(
        "claude",
        borg_core::sandbox::SandboxMode::Direct,
        "test-image",
    );
    use borg_core::agent::AgentBackend;
    assert_eq!(backend.name(), "claude");
}

#[test]
fn reliable_backend_preserves_name() {
    use borg_core::agent::AgentBackend;
    let inner = std::sync::Arc::new(
        borg_agent::AgentSdkBackend::new(ProviderConfig::Subscription),
    );
    let reliable = borg_agent::ReliableBackend::wrap(inner);
    assert_eq!(reliable.name(), "agent-sdk");
}

#[test]
fn reliable_backend_preserves_capabilities() {
    use borg_core::agent::AgentBackend;
    let inner = std::sync::Arc::new(
        borg_agent::AgentSdkBackend::new(ProviderConfig::Subscription),
    );
    let reliable = borg_agent::ReliableBackend::wrap(inner);
    let caps = reliable.capabilities();
    assert!(caps.supports_mcp);
}

// ── Backend selection tests ──────────────────────────────────────────────

#[test]
fn backend_map_contains_expected_names() {
    let mut backends: HashMap<String, std::sync::Arc<dyn borg_core::agent::AgentBackend>> =
        HashMap::new();
    backends.insert(
        "agent-sdk".into(),
        std::sync::Arc::new(borg_agent::AgentSdkBackend::new(ProviderConfig::Subscription)),
    );
    backends.insert(
        "claude".into(),
        std::sync::Arc::new(borg_agent::claude::ClaudeBackend::new(
            "claude",
            borg_core::sandbox::SandboxMode::Direct,
            "test",
        )),
    );

    let registry = borg_core::registry::PluginRegistry::new(backends);
    assert!(registry.get_backend("agent-sdk").is_some());
    assert!(registry.get_backend("claude").is_some());
    assert!(registry.get_backend("nonexistent").is_none());
    assert_eq!(registry.default_backend().unwrap().name(), "agent-sdk");
}
