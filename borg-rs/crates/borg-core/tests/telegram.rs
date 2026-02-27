/// Tests for `check_mentions_bot` — the extracted helper that determines whether
/// a Telegram message mentions the bot.
///
/// Expected signature (to be added to telegram.rs by the impl agent):
///
///   pub fn check_mentions_bot(bot_username: &str, text: &str, entities: &[serde_json::Value]) -> bool
///
/// The function must:
///   1. Return true if `text` (lowercased) contains `@{bot_username}` (text fallback).
///   2. Return true if any entity with type=="mention" refers to the bot username,
///      using the entity's `offset` and `length` (UTF-16 code-unit indices) to slice
///      the mention from `text`, stripping the leading '@', and comparing
///      case-insensitively to `bot_username`.
///   3. Return false when neither condition holds.
use borg_core::telegram::check_mentions_bot;
use serde_json::json;

fn mention_entity(offset: u64, length: u64) -> serde_json::Value {
    json!({ "type": "mention", "offset": offset, "length": length })
}

fn text_mention_entity(offset: u64, length: u64) -> serde_json::Value {
    // text_mention has a different type — should not be matched by the mention path
    json!({ "type": "text_mention", "offset": offset, "length": length })
}

// ── Acceptance criteria ────────────────────────────────────────────────────

/// AC1: @alice entity, bot is @borgbot → false
#[test]
fn other_user_mention_not_bot() {
    // "@alice hello" — entity covers "@alice" (offset 0, length 6)
    let entities = [mention_entity(0, 6)];
    assert!(!check_mentions_bot("borgbot", "@alice hello", &entities));
}

/// AC2: @borgbot entity → true
#[test]
fn bot_mention_exact() {
    // "@borgbot do this" — entity covers "@borgbot" (offset 0, length 8)
    let entities = [mention_entity(0, 8)];
    assert!(check_mentions_bot("borgbot", "@borgbot do this", &entities));
}

/// AC3: @BorgBot entity (mixed case) → true (case-insensitive)
#[test]
fn bot_mention_case_insensitive() {
    let entities = [mention_entity(0, 8)];
    assert!(check_mentions_bot("borgbot", "@BorgBot do this", &entities));
}

/// AC4: no entities, text contains "@borgbot" literal → true (text fallback)
#[test]
fn text_fallback_no_entities() {
    assert!(check_mentions_bot("borgbot", "hey @borgbot help me", &[]));
}

/// AC5: both @alice and @borgbot entities → true
#[test]
fn multiple_entities_includes_bot() {
    // "@alice and @borgbot" — alice: offset 0 len 6, borgbot: offset 11 len 8
    let entities = [mention_entity(0, 6), mention_entity(11, 8)];
    assert!(check_mentions_bot(
        "borgbot",
        "@alice and @borgbot",
        &entities
    ));
}

/// AC6: @alice entity only, no bot text anywhere → false
#[test]
fn other_user_only_no_bot_text() {
    let entities = [mention_entity(0, 6)];
    assert!(!check_mentions_bot("borgbot", "@alice hello", &entities));
}

/// AC7: no entities at all, text has @borgbot → true (text-contains is sole signal)
#[test]
fn no_entities_text_contains_bot() {
    assert!(check_mentions_bot("borgbot", "ping @borgbot please", &[]));
}

/// AC7 (negative): no entities, text has no bot name → false
#[test]
fn no_entities_no_bot_text() {
    assert!(!check_mentions_bot("borgbot", "hello world", &[]));
}

// ── Edge cases ─────────────────────────────────────────────────────────────

/// Zero-length mention entity must not panic and must return false.
#[test]
fn zero_length_entity_no_panic() {
    let entities = [mention_entity(0, 0)];
    // text-fallback also does not fire because text is "@borgbot" but
    // the entity path must not panic; the overall result is true only via
    // text-fallback — so use a text that does NOT contain the bot name to
    // isolate the entity-path guard.
    assert!(!check_mentions_bot("borgbot", "@alice", &entities));
}

/// Offset beyond text length must not panic and must return false.
#[test]
fn offset_beyond_text_no_panic() {
    let entities = [mention_entity(999, 8)];
    assert!(!check_mentions_bot("borgbot", "short", &entities));
}

/// non-mention entity types (e.g. text_mention) are ignored by the entity path;
/// the text also doesn't contain the bot name, so result is false.
#[test]
fn non_mention_entity_type_ignored() {
    // text is "@alice" not "@borgbot", so text-fallback won't fire either
    let entities = [text_mention_entity(0, 6)];
    assert!(!check_mentions_bot("borgbot", "@alice", &entities));
}

/// Unicode text: emoji before the mention shifts byte offsets but Telegram
/// uses UTF-16 code-unit offsets.  The function must slice correctly.
///
/// Example: "💥 @borgbot" — the emoji is 2 UTF-16 code units (U+1F4A5 is a
/// surrogate pair), so the mention "@borgbot" (8 chars) starts at UTF-16
/// offset 3 (2 for emoji + 1 for space).
#[test]
fn unicode_utf16_offset_emoji_before_mention() {
    // "💥 @borgbot"
    // UTF-16: [0xD83D, 0xDCA5, 0x0020, 0x0040, 0x0062, 0x006F, 0x0072, 0x0067, 0x0062, 0x006F, 0x0074]
    //          ^^^emoji (2 units)^^^  space   '@'   'b'   'o'   'r'   'g'   'b'   'o'   't'
    // offset=3, length=8
    let entities = [mention_entity(3, 8)];
    assert!(check_mentions_bot("borgbot", "💥 @borgbot", &entities));
}

/// When bot_username is empty (connect() not called), entity path always
/// returns false for any mention. Text-fallback path is unaffected.
#[test]
fn empty_bot_username_entity_path_false() {
    // "@borgbot" entity but no configured username → entity path: false
    // text fallback: text_lower.contains("@") where bot_name="@" → would
    // match everything; this is pre-existing behaviour, so we only assert
    // the entity path doesn't incorrectly claim a match.
    // Use a text with no '@' at all to isolate entity path.
    let entities = [mention_entity(0, 8)];
    assert!(!check_mentions_bot("", "no at sign here", &entities));
}
