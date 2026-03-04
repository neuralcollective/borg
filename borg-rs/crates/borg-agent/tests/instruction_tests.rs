use borg_agent::instruction::build_knowledge_section;
use borg_core::db::KnowledgeFile;

fn kf(file_name: &str, description: &str, inline: bool) -> KnowledgeFile {
    KnowledgeFile {
        id: 1,
        file_name: file_name.to_string(),
        description: description.to_string(),
        size_bytes: 0,
        inline,
        tags: String::new(),
        category: String::new(),
        jurisdiction: String::new(),
        project_id: None,
        created_at: String::new(),
    }
}

fn write_temp(dir: &std::path::Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

// ── Empty list ────────────────────────────────────────────────────────────

#[test]
fn test_empty_files_produces_no_section() {
    let result = build_knowledge_section(&[], "/knowledge");
    assert!(result.is_empty(), "expected empty string for no files, got: {result:?}");
}

// ── External files ────────────────────────────────────────────────────────

#[test]
fn test_external_file_renders_as_link() {
    let files = vec![kf("guide.md", "", false)];
    let result = build_knowledge_section(&files, "/knowledge");
    assert!(result.contains("## Knowledge Base"));
    assert!(result.contains("`/knowledge/guide.md`"));
}

#[test]
fn test_external_file_with_description() {
    let files = vec![kf("style.md", "Style guidelines", false)];
    let result = build_knowledge_section(&files, "/knowledge");
    assert!(result.contains("`/knowledge/style.md`"));
    assert!(result.contains("Style guidelines"));
}

#[test]
fn test_external_file_no_fenced_block() {
    let files = vec![kf("ref.md", "", false)];
    let result = build_knowledge_section(&files, "/knowledge");
    assert!(!result.contains("```"), "external file should not have fenced code block");
}

// ── Inline files ──────────────────────────────────────────────────────────

#[test]
fn test_inline_file_embedded_in_fenced_block() {
    let dir = tempdir();
    write_temp(&dir, "rules.md", "Rule 1: be nice\nRule 2: be clear");

    let files = vec![kf("rules.md", "", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    assert!(result.contains("## Knowledge Base"));
    assert!(result.contains("**rules.md**"));
    assert!(result.contains("```"));
    assert!(result.contains("Rule 1: be nice"));
    assert!(result.contains("Rule 2: be clear"));
}

#[test]
fn test_inline_file_with_description_in_parens() {
    let dir = tempdir();
    write_temp(&dir, "notes.md", "some content");

    let files = vec![kf("notes.md", "Meeting notes", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    assert!(result.contains("(Meeting notes)"));
}

#[test]
fn test_inline_empty_file_renders_as_plain_listing() {
    let dir = tempdir();
    write_temp(&dir, "empty.md", "   ");

    let files = vec![kf("empty.md", "", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    assert!(result.contains("**empty.md**"));
    assert!(!result.contains("```"), "empty inline file should not have fenced block");
}

#[test]
fn test_inline_missing_file_renders_as_plain_listing() {
    // If the file doesn't exist, read_to_string returns "" → treated as empty
    let files = vec![kf("missing.md", "", true)];
    let result = build_knowledge_section(&files, "/nonexistent/path");

    assert!(result.contains("**missing.md**"));
    assert!(!result.contains("```"));
}

// ── Mixed inline + external ───────────────────────────────────────────────

#[test]
fn test_mixed_inline_and_external() {
    let dir = tempdir();
    write_temp(&dir, "inline.md", "Inline content here");

    let files = vec![
        kf("inline.md", "Embedded file", true),
        kf("external.md", "Linked file", false),
    ];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    // Single knowledge section header
    assert_eq!(result.matches("## Knowledge Base").count(), 1);

    // Inline: embedded with fenced block
    assert!(result.contains("**inline.md**"));
    assert!(result.contains("Inline content here"));
    assert!(result.contains("```"));

    // External: link format
    assert!(result.contains("`/knowledge/external.md`"));
    assert!(result.contains("Linked file"));

    // The link format should not appear for the inline file
    assert!(!result.contains("`/knowledge/inline.md`"));
}

#[test]
fn test_mixed_order_preserved() {
    let dir = tempdir();
    write_temp(&dir, "first.md", "Alpha");

    let files = vec![
        kf("first.md", "", true),
        kf("second.md", "", false),
        kf("third.md", "", false),
    ];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    let pos_first = result.find("first.md").unwrap();
    let pos_second = result.find("second.md").unwrap();
    let pos_third = result.find("third.md").unwrap();
    assert!(pos_first < pos_second, "first should appear before second");
    assert!(pos_second < pos_third, "second should appear before third");
}

// ── Helper ────────────────────────────────────────────────────────────────

fn tempdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "borg_instr_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
