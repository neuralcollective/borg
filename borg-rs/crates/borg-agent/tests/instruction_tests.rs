use borg_agent::instruction::build_knowledge_section;
use borg_core::db::KnowledgeFile;
use std::fs;

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

#[test]
fn test_empty_files_returns_empty_string() {
    let result = build_knowledge_section(&[], "/some/dir");
    assert!(result.is_empty());
}

#[test]
fn test_non_inline_file_with_description() {
    let files = [kf("guide.md", "Style guide", false)];
    let result = build_knowledge_section(&files, "/some/dir");
    assert!(result.contains("## Knowledge Base"));
    assert!(result.contains("- `/knowledge/guide.md`: Style guide"));
}

#[test]
fn test_non_inline_file_no_description() {
    let files = [kf("notes.txt", "", false)];
    let result = build_knowledge_section(&files, "/some/dir");
    // Entry line should not have ": " after the filename when description is empty
    assert!(result.contains("- `/knowledge/notes.txt`\n"));
    assert!(!result.contains("- `/knowledge/notes.txt`:"));
}

#[test]
fn test_inline_file_with_content() {
    let dir = tempfile_dir("borg_kf_content");
    fs::write(dir.join("facts.txt"), "Important facts here.").unwrap();
    let files = [kf("facts.txt", "Key facts", true)];
    let result = build_knowledge_section(&files, &dir.to_string_lossy());
    assert!(result.contains("- **facts.txt** (Key facts):"));
    assert!(result.contains("```\nImportant facts here.\n```"));
}

#[test]
fn test_inline_file_empty_on_disk() {
    let dir = tempfile_dir("borg_kf_empty");
    fs::write(dir.join("empty.txt"), "").unwrap();
    let files = [kf("empty.txt", "An empty file", true)];
    let result = build_knowledge_section(&files, &dir.to_string_lossy());
    assert!(result.contains("- **empty.txt**: An empty file"));
    assert!(!result.contains("```"));
}

#[test]
fn test_inline_file_missing_on_disk_treated_as_empty() {
    let dir = tempfile_dir("borg_kf_missing");
    let files = [kf("missing.txt", "Not on disk", true)];
    let result = build_knowledge_section(&files, &dir.to_string_lossy());
    assert!(result.contains("- **missing.txt**: Not on disk"));
    assert!(!result.contains("```"));
}

#[test]
fn test_multiple_files_all_entries_present() {
    let dir = tempfile_dir("borg_kf_multi");
    fs::write(dir.join("inline.md"), "Inline content.").unwrap();
    let files = [
        kf("inline.md", "Inline doc", true),
        kf("ref.pdf", "Reference PDF", false),
    ];
    let result = build_knowledge_section(&files, &dir.to_string_lossy());
    assert!(result.contains("- **inline.md** (Inline doc):"));
    assert!(result.contains("Inline content."));
    assert!(result.contains("- `/knowledge/ref.pdf`: Reference PDF"));
}

/// Returns a path to a fresh temp subdirectory named by `tag`.
fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("{}_{}", tag, std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
