use borg_core::pipeline::truncate_str;

// ASCII: truncated at limit
#[test]
fn truncates_ascii_at_char_limit() {
    let s = "abcdef";
    assert_eq!(truncate_str(s, 3), "abc");
}

// ASCII: shorter than limit — returned unchanged
#[test]
fn no_truncation_when_shorter_than_limit() {
    let s = "hi";
    assert_eq!(truncate_str(s, 100), "hi");
}

// ASCII: exactly at limit — returned unchanged
#[test]
fn no_truncation_at_exact_limit() {
    let s = "abc";
    assert_eq!(truncate_str(s, 3), "abc");
}

// Empty string — must not panic
#[test]
fn empty_string_returns_empty() {
    assert_eq!(truncate_str("", 10), "");
}

// 2-byte chars (é = U+00E9): byte boundary falls between chars
// "éàü" is 6 bytes; truncating to 2 chars must give "éà" (4 bytes), not panic.
#[test]
fn truncates_two_byte_chars_safely() {
    let s = "éàü";
    let result = truncate_str(s, 2);
    assert_eq!(result, "éà");
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
}

// 3-byte chars (CJK): "中文测试" truncated to 2 chars must give "中文", not panic.
#[test]
fn truncates_cjk_chars_safely() {
    let s = "中文测试";
    let result = truncate_str(s, 2);
    assert_eq!(result, "中文");
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
}

// 4-byte chars (emoji): "🦀🔥💥" truncated to 1 char gives "🦀"
#[test]
fn truncates_emoji_chars_safely() {
    let s = "🦀🔥💥";
    let result = truncate_str(s, 1);
    assert_eq!(result, "🦀");
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
}

// Mixed ASCII + multibyte: limit falls inside multi-byte cluster
// "hello 日本語" truncated to 8 chars = "hello 日本"
#[test]
fn truncates_mixed_string_at_char_boundary() {
    let s = "hello 日本語";
    let result = truncate_str(s, 8);
    assert_eq!(result, "hello 日本");
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
}

// Simulates the run_integration usage with 300-char limit on a CJK error message
#[test]
fn run_integration_300_char_limit_is_safe_for_cjk() {
    // 400 CJK characters = 1200 bytes; byte-slicing at 300 would panic
    let long_cjk: String = "日".repeat(400);
    let result = truncate_str(&long_cjk, 300);
    assert_eq!(result.chars().count(), 300);
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
}

// Simulates the run_integration usage with 200-char limit
#[test]
fn run_integration_200_char_limit_is_safe_for_accented() {
    // 300 é chars = 600 bytes; byte-slicing at 200 would panic
    let long_accented: String = "é".repeat(300);
    let result = truncate_str(&long_accented, 200);
    assert_eq!(result.chars().count(), 200);
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
}

// Simulates the self-update build failure with 500-char limit
#[test]
fn self_update_500_char_limit_is_safe_for_multibyte() {
    let long: String = "ñ".repeat(600);
    let result = truncate_str(&long, 500);
    assert_eq!(result.chars().count(), 500);
    assert!(std::str::from_utf8(result.as_bytes()).is_ok());
}
