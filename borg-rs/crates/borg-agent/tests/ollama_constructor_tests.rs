use borg_agent::ollama::OllamaBackend;

#[test]
fn new_returns_ok_with_valid_args() {
    let backend = OllamaBackend::new("http://localhost:11434", "llama3.2");
    assert!(backend.is_ok());
}

#[test]
fn new_stores_base_url_and_model() {
    let backend = OllamaBackend::new("http://myhost:11434", "mistral").unwrap();
    assert_eq!(backend.base_url, "http://myhost:11434");
    assert_eq!(backend.model, "mistral");
}

#[test]
fn with_timeout_returns_ok() {
    let backend = OllamaBackend::new("http://localhost:11434", "llama3.2")
        .unwrap()
        .with_timeout(60);
    assert!(backend.is_ok());
}

#[test]
fn with_timeout_updates_timeout_secs() {
    let backend = OllamaBackend::new("http://localhost:11434", "llama3.2")
        .unwrap()
        .with_timeout(42)
        .unwrap();
    assert_eq!(backend.timeout_secs, 42);
}
