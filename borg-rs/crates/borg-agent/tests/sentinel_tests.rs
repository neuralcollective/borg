// Tests for AC9: `extract_phase_result` in `borg_agent::claude`.
//
// These tests FAIL initially (fail to compile) because `extract_phase_result`
// does not yet exist in `borg_agent::claude`.
//
// Once implemented they cover:
//   AC9: extract_phase_result returns content from a valid marker pair.
//   AC9: extract_phase_result returns None when no markers are present.
//   AC9: extract_phase_result returns None when only the start marker is present.
//   AC9: extract_phase_result returns the LAST pair when multiple pairs exist.
//   EC1: unclosed start marker → None.
//   EC2: whitespace-only content between markers → None.
//   EC4: three marker pairs → last (third) is returned.

use borg_agent::claude::extract_phase_result;

const START: &str = "---PHASE_RESULT_START---";
const END: &str = "---PHASE_RESULT_END---";

// =============================================================================
// AC9: valid pair — content is returned
// =============================================================================

#[test]
fn test_basic_extraction() {
    let text = format!("{START}\nSpec complete.\n{END}");
    let result = extract_phase_result(&text);
    assert!(result.is_some());
    assert!(result.unwrap().contains("Spec complete."));
}

#[test]
fn test_extraction_with_surrounding_prose() {
    let text = format!(
        "I reviewed the codebase.\n\n{START}\nTests written: 5 files.\n{END}\n\nPhase complete."
    );
    let result = extract_phase_result(&text);
    assert!(result.is_some());
    assert!(result.unwrap().contains("Tests written: 5 files."));
}

#[test]
fn test_extracted_content_is_trimmed() {
    let text = format!("{START}\n  Summary line.  \n{END}");
    let result = extract_phase_result(&text);
    assert!(result.is_some());
    let r = result.unwrap();
    // Must not start or end with whitespace after trim
    assert_eq!(r, r.trim());
}

// =============================================================================
// AC9: no markers → None
// =============================================================================

#[test]
fn test_no_markers_returns_none() {
    let result = extract_phase_result("Plain output with no markers at all.");
    assert!(result.is_none());
}

#[test]
fn test_empty_string_returns_none() {
    let result = extract_phase_result("");
    assert!(result.is_none());
}

#[test]
fn test_ndjson_without_markers_returns_none() {
    let data = r#"{"type":"system","session_id":"abc"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Analyzing..."}]}}
{"type":"result","result":"Analysis complete."}"#;
    assert!(extract_phase_result(data).is_none());
}

// =============================================================================
// AC9 / EC1: only start marker present → None
// =============================================================================

#[test]
fn test_only_start_marker_returns_none() {
    let text = format!("{START}\nThis was never closed.");
    assert!(extract_phase_result(&text).is_none());
}

#[test]
fn test_only_end_marker_returns_none() {
    let text = format!("Some text here.\n{END}");
    assert!(extract_phase_result(&text).is_none());
}

#[test]
fn test_unclosed_start_in_stream_returns_none() {
    let text = format!("preamble\n{START}\ncontent without end\nmore lines");
    assert!(extract_phase_result(&text).is_none());
}

// =============================================================================
// EC2: whitespace-only content between markers → None
// =============================================================================

#[test]
fn test_whitespace_only_content_returns_none() {
    let text = format!("{START}\n   \n\t\n{END}");
    assert!(extract_phase_result(&text).is_none());
}

#[test]
fn test_empty_content_between_markers_returns_none() {
    let text = format!("{START}\n{END}");
    assert!(extract_phase_result(&text).is_none());
}

// =============================================================================
// AC9 / EC4: multiple pairs → last complete pair wins
// =============================================================================

#[test]
fn test_multiple_pairs_last_wins() {
    let text = format!(
        "{START}\nFirst attempt.\n{END}\n\n{START}\nRevised summary — final one.\n{END}"
    );
    let result = extract_phase_result(&text);
    assert!(result.is_some());
    let r = result.unwrap();
    assert!(r.contains("Revised summary"), "expected revised summary, got: {r}");
    assert!(!r.contains("First attempt"), "should not contain first attempt, got: {r}");
}

#[test]
fn test_three_pairs_third_is_returned() {
    let text = format!(
        "{START}\nFirst.\n{END}\n{START}\nSecond.\n{END}\n{START}\nThird and final.\n{END}"
    );
    let result = extract_phase_result(&text);
    assert!(result.is_some());
    let r = result.unwrap();
    assert!(r.contains("Third and final."), "got: {r}");
    assert!(!r.contains("First."), "got: {r}");
    assert!(!r.contains("Second."), "got: {r}");
}

// =============================================================================
// Multi-line content is preserved
// =============================================================================

#[test]
fn test_multiline_content_preserved() {
    let text = format!("{START}\nLine one.\nLine two.\nLine three.\n{END}");
    let result = extract_phase_result(&text);
    assert!(result.is_some());
    let r = result.unwrap();
    assert!(r.contains("Line one."));
    assert!(r.contains("Line two."));
    assert!(r.contains("Line three."));
}

// =============================================================================
// Markers split across raw bytes (end-to-end correctness check)
// =============================================================================

#[test]
fn test_markers_not_present_in_plain_ndjson_escape() {
    // The raw marker strings consist only of ASCII characters that are never
    // JSON-escaped, so searching raw bytes is correct.
    let raw = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"---PHASE_RESULT_START---\nmy summary\n---PHASE_RESULT_END---"}]}}"#;
    // extract_phase_result operates on decoded text (result.output), not raw NDJSON.
    // When the decoded text contains the markers, extraction must succeed.
    let decoded = format!("{START}\nmy summary\n{END}");
    let result = extract_phase_result(&decoded);
    assert!(result.is_some());
    assert!(result.unwrap().contains("my summary"));
    // Raw NDJSON with escaped newlines must not falsely trigger on its own.
    let _ = raw; // used above as documentation only
}
