use borg_agent::instruction::build_knowledge_section;
use borg_core::db::KnowledgeFile;
use std::path::PathBuf;
use tracing_test::traced_test;

fn kf(id: i64, file_name: &str, description: &str, inline: bool) -> KnowledgeFile {
    KnowledgeFile {
        id,
        file_name: file_name.to_string(),
        description: description.to_string(),
        size_bytes: 0,
        inline,
        created_at: String::new(),
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("borg_ks_{}", label));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

// =============================================================================
// Empty input
// =============================================================================

#[test]
fn empty_files_returns_empty_string() {
    assert!(build_knowledge_section(&[], "/any/path").is_empty());
}

// =============================================================================
// Reference (non-inline) files
// =============================================================================

#[test]
fn reference_file_no_description() {
    let files = vec![kf(1, "guide.md", "", false)];
    let result = build_knowledge_section(&files, "/any");
    assert!(result.contains("- `/knowledge/guide.md`\n"));
    assert!(!result.contains(": "));
}

#[test]
fn reference_file_with_description() {
    let files = vec![kf(1, "guide.md", "User guide", false)];
    let result = build_knowledge_section(&files, "/any");
    assert!(result.contains("- `/knowledge/guide.md`: User guide\n"));
}

// =============================================================================
// Inline files — file has content
// =============================================================================

#[test]
fn inline_file_with_content_no_description() {
    let dir = temp_dir("inline_no_desc");
    std::fs::write(dir.join("facts.txt"), "Some fact here").unwrap();

    let files = vec![kf(1, "facts.txt", "", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(result.contains("- **facts.txt**:\n```\nSome fact here\n```\n"));
    // No parenthesised description section
    assert!(!result.contains("()"));
}

#[test]
fn inline_file_with_content_and_description() {
    let dir = temp_dir("inline_with_desc");
    std::fs::write(dir.join("facts.txt"), "Some fact here").unwrap();

    let files = vec![kf(1, "facts.txt", "Factual info", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(result.contains("- **facts.txt** (Factual info):\n```\nSome fact here\n```\n"));
}

// =============================================================================
// Inline files — empty / whitespace-only content falls back to name-only
// =============================================================================

#[test]
fn inline_empty_file_no_description_falls_back_to_name_only() {
    let dir = temp_dir("inline_empty_no_desc");
    std::fs::write(dir.join("empty.txt"), "").unwrap();

    let files = vec![kf(1, "empty.txt", "", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(result.contains("- **empty.txt**\n"));
    assert!(!result.contains("```"));
}

#[test]
fn inline_whitespace_only_file_with_description_falls_back() {
    let dir = temp_dir("inline_empty_with_desc");
    std::fs::write(dir.join("empty.txt"), "   \n  ").unwrap();

    let files = vec![kf(1, "empty.txt", "Should be listed", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(result.contains("- **empty.txt**: Should be listed\n"));
    assert!(!result.contains("```"));
}

#[test]
fn inline_missing_file_falls_back_to_name_only() {
    let files = vec![kf(1, "nonexistent.txt", "", true)];
    let result = build_knowledge_section(&files, "/nonexistent/path");
    assert!(result.contains("- **nonexistent.txt**\n"));
    assert!(!result.contains("```"));
}

#[traced_test]
#[test]
fn inline_missing_file_emits_warn_log() {
    let files = vec![kf(1, "nonexistent.txt", "", true)];
    let _ = build_knowledge_section(&files, "/nonexistent/path");
    assert!(logs_contain("failed to read knowledge file"));
    assert!(logs_contain("nonexistent.txt"));
}

// =============================================================================
// Multiple files
// =============================================================================

#[test]
fn multiple_files_produce_correct_multi_entry_output() {
    let dir = temp_dir("multi");
    std::fs::write(dir.join("a.txt"), "Content A").unwrap();

    let files = vec![
        kf(1, "a.txt", "File A", true),
        kf(2, "b.md", "File B", false),
    ];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(result.starts_with("## Knowledge Base\n"));
    assert!(result.contains("- **a.txt** (File A):\n```\nContent A\n```\n"));
    assert!(result.contains("- `/knowledge/b.md`: File B\n"));
}

#[test]
fn multiple_reference_files_all_listed() {
    let files = vec![
        kf(1, "alpha.md", "", false),
        kf(2, "beta.md", "Beta doc", false),
        kf(3, "gamma.md", "Gamma doc", false),
    ];
    let result = build_knowledge_section(&files, "/any");
    assert!(result.contains("- `/knowledge/alpha.md`\n"));
    assert!(result.contains("- `/knowledge/beta.md`: Beta doc\n"));
    assert!(result.contains("- `/knowledge/gamma.md`: Gamma doc\n"));
}
