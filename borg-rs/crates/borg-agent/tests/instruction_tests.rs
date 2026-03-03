use borg_agent::instruction::build_knowledge_section;
use borg_core::db::KnowledgeFile;
use std::fs;

fn make_file(id: i64, file_name: &str, description: &str, inline: bool) -> KnowledgeFile {
    KnowledgeFile {
        id,
        file_name: file_name.to_string(),
        description: description.to_string(),
        size_bytes: 0,
        inline,
        created_at: String::new(),
    }
}

// =============================================================================
// Empty files list → empty string
// =============================================================================

#[test]
fn test_empty_files_returns_empty() {
    let result = build_knowledge_section(&[], "/any/dir");
    assert!(result.is_empty());
}

// =============================================================================
// Non-inline files → /knowledge/<name> reference lines
// =============================================================================

#[test]
fn test_non_inline_no_description() {
    let files = [make_file(1, "guide.pdf", "", false)];
    let result = build_knowledge_section(&files, "/any/dir");
    assert!(result.contains("- `/knowledge/guide.pdf`\n"), "got: {result}");
}

#[test]
fn test_non_inline_with_description() {
    let files = [make_file(1, "guide.pdf", "The style guide", false)];
    let result = build_knowledge_section(&files, "/any/dir");
    assert!(
        result.contains("- `/knowledge/guide.pdf`: The style guide\n"),
        "got: {result}"
    );
}

// =============================================================================
// Inline files with content → fenced code block
// =============================================================================

#[test]
fn test_inline_with_content_emits_code_block() {
    let dir = std::env::temp_dir();
    let file_path = dir.join("borg_kb_test_content.txt");
    fs::write(&file_path, "Hello world\n").unwrap();

    let files = [make_file(1, "borg_kb_test_content.txt", "", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    fs::remove_file(&file_path).ok();

    assert!(
        result.contains("**borg_kb_test_content.txt**"),
        "got: {result}"
    );
    assert!(result.contains("```\nHello world\n```"), "got: {result}");
}

#[test]
fn test_inline_with_content_and_description_uses_parens() {
    let dir = std::env::temp_dir();
    let file_path = dir.join("borg_kb_test_desc.txt");
    fs::write(&file_path, "content here").unwrap();

    let files = [make_file(1, "borg_kb_test_desc.txt", "My description", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    fs::remove_file(&file_path).ok();

    assert!(result.contains("(My description)"), "got: {result}");
    assert!(result.contains("```\ncontent here\n```"), "got: {result}");
}

// =============================================================================
// Inline files with empty/whitespace content → reference fallback
// =============================================================================

#[test]
fn test_inline_empty_file_falls_back_to_reference() {
    let dir = std::env::temp_dir();
    let file_path = dir.join("borg_kb_test_empty.txt");
    fs::write(&file_path, "").unwrap();

    let files = [make_file(1, "borg_kb_test_empty.txt", "", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    fs::remove_file(&file_path).ok();

    assert!(result.contains("**borg_kb_test_empty.txt**"), "got: {result}");
    assert!(!result.contains("```"), "should not contain code fence, got: {result}");
}

#[test]
fn test_inline_whitespace_only_falls_back_to_reference() {
    let dir = std::env::temp_dir();
    let file_path = dir.join("borg_kb_test_ws.txt");
    fs::write(&file_path, "   \n\t  \n  ").unwrap();

    let files = [make_file(1, "borg_kb_test_ws.txt", "", true)];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    fs::remove_file(&file_path).ok();

    assert!(result.contains("**borg_kb_test_ws.txt**"), "got: {result}");
    assert!(!result.contains("```"), "should not contain code fence, got: {result}");
}

#[test]
fn test_inline_missing_file_falls_back_to_reference() {
    let files = [make_file(1, "nonexistent_kb.txt", "a description", true)];
    let result = build_knowledge_section(&files, "/nonexistent/dir");

    assert!(result.contains("**nonexistent_kb.txt**"), "got: {result}");
    assert!(result.contains(": a description"), "got: {result}");
    assert!(!result.contains("```"), "should not contain code fence, got: {result}");
}

// =============================================================================
// Multiple mixed files emitted in order
// =============================================================================

#[test]
fn test_multiple_mixed_files_emitted_in_order() {
    let dir = std::env::temp_dir();
    let file_path = dir.join("borg_kb_test_mixed_inline.txt");
    fs::write(&file_path, "inline content").unwrap();

    let files = [
        make_file(1, "ref_doc.pdf", "A reference doc", false),
        make_file(2, "borg_kb_test_mixed_inline.txt", "Inline doc", true),
        make_file(3, "ref2.pdf", "", false),
    ];
    let result = build_knowledge_section(&files, dir.to_str().unwrap());

    fs::remove_file(&file_path).ok();

    assert!(result.contains("`/knowledge/ref_doc.pdf`"), "got: {result}");
    assert!(
        result.contains("**borg_kb_test_mixed_inline.txt**"),
        "got: {result}"
    );
    assert!(result.contains("`/knowledge/ref2.pdf`"), "got: {result}");

    let pos_ref1 = result.find("ref_doc.pdf").unwrap();
    let pos_inline = result.find("borg_kb_test_mixed_inline.txt").unwrap();
    let pos_ref2 = result.find("ref2.pdf").unwrap();
    assert!(pos_ref1 < pos_inline, "ref_doc should come before inline");
    assert!(pos_inline < pos_ref2, "inline should come before ref2");
}
