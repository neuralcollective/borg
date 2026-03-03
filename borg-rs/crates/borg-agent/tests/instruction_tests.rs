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

// ── Empty input ───────────────────────────────────────────────────────────────

#[test]
fn empty_files_returns_empty_string() {
    assert_eq!(build_knowledge_section(&[], "/any/dir"), "");
}

// ── inline = false (path-only) ────────────────────────────────────────────────

#[test]
fn path_only_no_description() {
    let files = [kf("api.md", "", false)];
    let out = build_knowledge_section(&files, "/ignored");
    assert!(out.contains("- `/knowledge/api.md`\n"), "got: {out}");
    assert!(!out.contains(": "), "should have no description, got: {out}");
}

#[test]
fn path_only_with_description() {
    let files = [kf("api.md", "API reference", false)];
    let out = build_knowledge_section(&files, "/ignored");
    assert!(
        out.contains("- `/knowledge/api.md`: API reference\n"),
        "got: {out}"
    );
}

// ── inline = true, file has content ──────────────────────────────────────────

#[test]
fn inline_with_content_embeds_code_fence() {
    let dir = tempdir("inline_content");
    write_temp(&dir, "notes.md", "hello world");
    let files = [kf("notes.md", "", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(out.contains("```\nhello world\n```"), "got: {out}");
    assert!(out.contains("**notes.md**"), "got: {out}");
}

#[test]
fn inline_with_content_and_description_uses_parens() {
    let dir = tempdir("inline_content_desc");
    write_temp(&dir, "notes.md", "some content");
    let files = [kf("notes.md", "meeting notes", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(
        out.contains("**notes.md** (meeting notes):"),
        "got: {out}"
    );
}

#[test]
fn inline_with_content_no_description_no_parens() {
    let dir = tempdir("inline_no_desc");
    write_temp(&dir, "notes.md", "content");
    let files = [kf("notes.md", "", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(!out.contains('('), "should have no parens, got: {out}");
    assert!(out.contains("**notes.md**:"), "got: {out}");
}

// ── inline = true, file is empty or missing ──────────────────────────────────

#[test]
fn inline_empty_file_falls_back_to_name_only() {
    let dir = tempdir("inline_empty");
    write_temp(&dir, "empty.md", "");
    let files = [kf("empty.md", "", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(out.contains("**empty.md**"), "got: {out}");
    assert!(!out.contains("```"), "should not embed code fence, got: {out}");
}

#[test]
fn inline_missing_file_falls_back_to_name_only() {
    let files = [kf("ghost.md", "", true)];
    let out = build_knowledge_section(&files, "/nonexistent/dir");
    assert!(out.contains("**ghost.md**"), "got: {out}");
    assert!(!out.contains("```"), "should not embed code fence, got: {out}");
}

#[test]
fn inline_empty_file_with_description_uses_colon_format() {
    let dir = tempdir("inline_empty_desc");
    write_temp(&dir, "empty.md", "   \n\t");
    let files = [kf("empty.md", "a desc", true)];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(out.contains("**empty.md**: a desc"), "got: {out}");
    assert!(!out.contains('('), "should not use parens format, got: {out}");
    assert!(!out.contains("```"), "should not embed code fence, got: {out}");
}

// ── Header is always included when files is non-empty ────────────────────────

#[test]
fn header_present_when_files_non_empty() {
    let files = [kf("x.md", "", false)];
    let out = build_knowledge_section(&files, "/ignored");
    assert!(out.starts_with("## Knowledge Base\n"), "got: {out}");
    assert!(
        out.contains("You have access to the following knowledge files at /knowledge/:\n"),
        "got: {out}"
    );
}

// ── Multiple files all rendered ───────────────────────────────────────────────

#[test]
fn multiple_files_all_appear() {
    let dir = tempdir("multi");
    write_temp(&dir, "b.md", "inline content");
    let files = [
        kf("a.md", "", false),
        kf("b.md", "desc b", true),
    ];
    let out = build_knowledge_section(&files, dir.to_str().unwrap());
    assert!(out.contains("- `/knowledge/a.md`"), "got: {out}");
    assert!(out.contains("**b.md** (desc b):"), "got: {out}");
    assert!(out.contains("inline content"), "got: {out}");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tempdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("borg_instr_test_{}", tag));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
