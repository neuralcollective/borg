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
fn test_empty_file_list_returns_empty_string() {
    assert!(build_knowledge_section(&[], "/any/dir").is_empty());
}

#[test]
fn test_non_inline_renders_path_with_description() {
    let files = vec![make_file("guide.md", "Usage guide", false)];
    let result = build_knowledge_section(&files, "/any/dir");
    assert!(result.contains("- `/knowledge/guide.md`: Usage guide\n"));
}

#[test]
fn test_non_inline_no_description_omits_colon() {
    let files = vec![make_file("guide.md", "", false)];
    let result = build_knowledge_section(&files, "/any/dir");
    assert!(result.contains("- `/knowledge/guide.md`\n"));
    assert!(!result.contains(": "));
}

#[test]
fn test_inline_with_content_renders_fenced_block() {
    let dir = std::env::temp_dir();
    let fname = "borg_test_inline_content.md";
    std::fs::write(dir.join(fname), "Hello world\nLine 2").unwrap();

    let files = vec![make_file(fname, "My doc", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    assert!(result.contains(&format!("**{fname}**")));
    assert!(result.contains("(My doc)"));
    assert!(result.contains("```\nHello world\nLine 2\n```"));

    let _ = std::fs::remove_file(dir.join(fname));
}

#[test]
fn test_inline_empty_content_fallback_to_list_entry() {
    let dir = std::env::temp_dir();
    let fname = "borg_test_inline_empty.md";
    std::fs::write(dir.join(fname), "   \n  ").unwrap();

    let files = vec![make_file(fname, "Empty doc", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    assert!(result.contains(&format!("**{fname}**")));
    assert!(result.contains(": Empty doc"));
    assert!(!result.contains("```"));

    let _ = std::fs::remove_file(dir.join(fname));
}

#[test]
fn test_inline_missing_file_fallback_to_list_entry() {
    // If the file doesn't exist on disk, read_to_string returns "" via unwrap_or_default.
    let files = vec![make_file("nonexistent.md", "Ghost file", true)];
    let result = build_knowledge_section(&files, "/nonexistent/dir");

    assert!(result.contains("**nonexistent.md**"));
    assert!(result.contains(": Ghost file"));
    assert!(!result.contains("```"));
}

#[test]
fn test_multiple_files_appear_in_order() {
    let dir = std::env::temp_dir();
    let fname_inline = "borg_test_multi_inline.md";
    std::fs::write(dir.join(fname_inline), "Inline content").unwrap();

    let files = vec![
        make_file("ref_first.md", "Reference file", false),
        make_file(fname_inline, "Inline file", true),
    ];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    let pos_ref = result.find("ref_first.md").expect("ref_first.md not found");
    let pos_inline = result.find(fname_inline).expect("inline file not found");
    assert!(pos_ref < pos_inline, "reference file should appear before inline file");

    let _ = std::fs::remove_file(dir.join(fname_inline));
}

#[test]
fn test_section_header_present_when_files_nonempty() {
    let files = vec![make_file("doc.md", "A doc", false)];
    let result = build_knowledge_section(&files, "/any");
    assert!(result.starts_with("## Knowledge Base\n"));
}
