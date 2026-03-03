use borg_agent::instruction::build_knowledge_section;
use borg_core::db::KnowledgeFile;

fn make_file(file_name: &str, description: &str, inline: bool) -> KnowledgeFile {
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

#[test]
fn test_empty_slice_returns_empty_string() {
    assert_eq!(build_knowledge_section(&[], "/knowledge"), "");
}

#[test]
fn test_non_inline_with_description() {
    let files = [make_file("rules.md", "Coding rules", false)];
    let result = build_knowledge_section(&files, "/irrelevant");
    assert!(
        result.contains("- `/knowledge/rules.md`: Coding rules\n"),
        "got: {result:?}"
    );
}

#[test]
fn test_non_inline_without_description_omits_colon() {
    let files = [make_file("guide.txt", "", false)];
    let result = build_knowledge_section(&files, "/irrelevant");
    let line = result.lines().find(|l| l.contains("guide.txt")).expect("line not found");
    assert_eq!(line, "- `/knowledge/guide.txt`", "got: {line:?}");
}

#[test]
fn test_inline_with_content_renders_fenced_block() {
    let dir = std::env::temp_dir();
    let file_name = "borg_ks_test_content.md";
    let file_path = dir.join(file_name);
    std::fs::write(&file_path, "# Hello\nThis is content.\n").expect("write temp file");

    let files = [make_file(file_name, "Test file", true)];
    let result = build_knowledge_section(&files, dir.to_str().expect("valid path"));

    let _ = std::fs::remove_file(&file_path);

    assert!(
        result.contains(&format!("- **{file_name}** (Test file):\n```\n")),
        "missing fenced header; got: {result:?}"
    );
    assert!(result.contains("# Hello"), "missing content; got: {result:?}");
    assert!(result.contains("\n```\n"), "missing closing fence; got: {result:?}");
}

#[test]
fn test_inline_file_not_found_renders_plain_entry() {
    let files = [make_file("nonexistent.md", "Missing file", true)];
    let result = build_knowledge_section(&files, "/no/such/dir");
    assert!(
        result.contains("- **nonexistent.md**: Missing file\n"),
        "got: {result:?}"
    );
    assert!(!result.contains("```"), "should not contain fence; got: {result:?}");
}

#[test]
fn test_inline_empty_description_omits_parens() {
    let dir = std::env::temp_dir();
    let file_name = "borg_ks_test_nodesc.md";
    let file_path = dir.join(file_name);
    std::fs::write(&file_path, "Some content here.\n").expect("write temp file");

    let files = [make_file(file_name, "", true)];
    let result = build_knowledge_section(&files, dir.to_str().expect("valid path"));

    let _ = std::fs::remove_file(&file_path);

    assert!(
        result.contains(&format!("- **{file_name}**:\n```\n")),
        "expected no parens; got: {result:?}"
    );
    assert!(!result.contains("()"), "empty parens present; got: {result:?}");
}
