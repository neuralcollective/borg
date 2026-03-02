use borg_agent::instruction::build_knowledge_section;
use borg_core::db::KnowledgeFile;

fn kf(file_name: &str, description: &str, inline: bool) -> KnowledgeFile {
    KnowledgeFile {
        id: 1,
        file_name: file_name.to_string(),
        description: description.to_string(),
        size_bytes: 0,
        inline,
        created_at: String::new(),
    }
}

#[test]
fn empty_files_returns_empty_string() {
    assert!(build_knowledge_section(&[], "/knowledge").is_empty());
}

#[test]
fn non_inline_produces_knowledge_path_line() {
    let result = build_knowledge_section(&[kf("guide.md", "Project guide", false)], "/any");
    assert!(result.contains("- `/knowledge/guide.md`"));
    assert!(result.contains("Project guide"));
}

#[test]
fn non_inline_no_description_omits_colon_suffix() {
    let result = build_knowledge_section(&[kf("guide.md", "", false)], "/any");
    assert!(result.contains("- `/knowledge/guide.md`"));
    assert!(!result.contains(": "));
}

#[test]
fn inline_missing_file_falls_back_to_name_only() {
    let result = build_knowledge_section(
        &[kf("missing.md", "Some desc", true)],
        "/nonexistent/dir/xyz",
    );
    assert!(result.contains("**missing.md**"));
    assert!(!result.contains("```"));
}

#[test]
fn inline_readable_content_embeds_fenced_code_block() {
    let dir = std::env::temp_dir();
    let file_name = "borg_test_knowledge_inline_readable.md";
    let path = dir.join(file_name);
    std::fs::write(&path, "# Hello\nThis is knowledge content.").unwrap();

    let result = build_knowledge_section(&[kf(file_name, "Test doc", true)], dir.to_str().unwrap());

    assert!(result.contains("```"));
    assert!(result.contains("# Hello"));
    assert!(result.contains("This is knowledge content."));

    std::fs::remove_file(&path).ok();
}
