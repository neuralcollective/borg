use borg_agent::instruction::build_knowledge_section;
use borg_core::db::KnowledgeFile;
use std::io::Write as _;

fn kf(file_name: &str, description: &str, inline: bool) -> KnowledgeFile {
    KnowledgeFile {
        id: 0,
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

fn write_temp_file(dir: &std::path::Path, name: &str, content: &str) {
    let mut f = std::fs::File::create(dir.join(name)).expect("create temp file");
    f.write_all(content.as_bytes()).expect("write temp file");
}

// ── empty list ────────────────────────────────────────────────────────────────

#[test]
fn empty_files_returns_empty_string() {
    assert_eq!(build_knowledge_section(&[], "/any/dir"), "");
}

// ── non-inline (path-only) ────────────────────────────────────────────────────

#[test]
fn non_inline_emits_knowledge_path_line() {
    let files = [kf("guide.pdf", "", false)];
    let out = build_knowledge_section(&files, "");
    assert!(out.contains("- `/knowledge/guide.pdf`\n"), "got: {out}");
}

#[test]
fn non_inline_with_description_appends_after_colon() {
    let files = [kf("guide.pdf", "Legal style guide", false)];
    let out = build_knowledge_section(&files, "");
    assert!(
        out.contains("- `/knowledge/guide.pdf`: Legal style guide\n"),
        "got: {out}"
    );
}

#[test]
fn non_inline_no_description_has_no_colon() {
    let files = [kf("ref.txt", "", false)];
    let out = build_knowledge_section(&files, "");
    assert!(!out.contains(": "), "unexpected colon in: {out}");
}

// ── inline with content ───────────────────────────────────────────────────────

#[test]
fn inline_with_content_emits_fenced_code_block() {
    let dir = tempdir();
    write_temp_file(&dir, "rules.md", "Rule one.\nRule two.");
    let files = [kf("rules.md", "", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(out.contains("- **rules.md**:\n```\n"), "got: {out}");
    assert!(out.contains("Rule one.\nRule two."), "got: {out}");
    assert!(out.contains("\n```\n"), "got: {out}");
}

#[test]
fn inline_with_content_and_description_uses_parens() {
    let dir = tempdir();
    write_temp_file(&dir, "glossary.txt", "Term: definition");
    let files = [kf("glossary.txt", "Legal glossary", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(
        out.contains("- **glossary.txt** (Legal glossary):\n```\n"),
        "got: {out}"
    );
}

#[test]
fn inline_with_content_no_description_has_no_parens() {
    let dir = tempdir();
    write_temp_file(&dir, "notes.txt", "Some notes.");
    let files = [kf("notes.txt", "", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(!out.contains("()"), "unexpected parens in: {out}");
}

// ── inline missing / empty → fallback bullet ─────────────────────────────────

#[test]
fn inline_missing_file_falls_back_to_name_bullet() {
    let files = [kf("missing.md", "", true)];
    let out = build_knowledge_section(&files, "/nonexistent/path");
    assert!(out.contains("- **missing.md**\n"), "got: {out}");
    assert!(!out.contains("```"), "should not have code fence: {out}");
}

#[test]
fn inline_missing_file_with_description_appends_after_colon() {
    let files = [kf("missing.md", "Important doc", true)];
    let out = build_knowledge_section(&files, "/nonexistent/path");
    assert!(
        out.contains("- **missing.md**: Important doc\n"),
        "got: {out}"
    );
}

#[test]
fn inline_empty_file_falls_back_to_name_bullet() {
    let dir = tempdir();
    write_temp_file(&dir, "empty.md", "   \n\t\n");
    let files = [kf("empty.md", "", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(out.contains("- **empty.md**\n"), "got: {out}");
    assert!(!out.contains("```"), "should not have code fence: {out}");
}

#[test]
fn inline_empty_file_with_description_appends_after_colon() {
    let dir = tempdir();
    write_temp_file(&dir, "empty.md", "");
    let files = [kf("empty.md", "Empty but described", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(
        out.contains("- **empty.md**: Empty but described\n"),
        "got: {out}"
    );
}

// ── header ────────────────────────────────────────────────────────────────────

#[test]
fn non_empty_list_includes_knowledge_base_header() {
    let files = [kf("x.pdf", "", false)];
    let out = build_knowledge_section(&files, "");
    assert!(out.starts_with("## Knowledge Base\n"), "got: {out}");
}

// ── multiple files ────────────────────────────────────────────────────────────

#[test]
fn multiple_files_all_emitted() {
    let dir = tempdir();
    write_temp_file(&dir, "inline.txt", "content here");
    let files = [
        kf("ref.pdf", "Reference", false),
        kf("inline.txt", "Notes", true),
        kf("absent.md", "Absent", true),
    ];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(out.contains("`/knowledge/ref.pdf`"), "got: {out}");
    assert!(out.contains("**inline.txt**"), "got: {out}");
    assert!(out.contains("**absent.md**"), "got: {out}");
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn tempdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "borg_ks_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}
