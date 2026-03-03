use borg_core::knowledge::chunk_text;

const CHUNK_SIZE: usize = 512;
const CHUNK_OVERLAP: usize = 64;

fn words(n: usize) -> String {
    (0..n).map(|i| format!("w{}", i)).collect::<Vec<_>>().join(" ")
}

#[test]
fn empty_string_returns_single_empty_chunk() {
    let chunks = chunk_text("");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], "");
}

#[test]
fn fewer_than_chunk_size_words_returns_single_chunk() {
    let text = words(100);
    let chunks = chunk_text(&text);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].split_whitespace().count(), 100);
}

#[test]
fn exactly_chunk_size_words_returns_single_chunk() {
    let text = words(CHUNK_SIZE);
    let chunks = chunk_text(&text);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].split_whitespace().count(), CHUNK_SIZE);
}

#[test]
fn chunk_size_plus_one_returns_two_chunks() {
    let text = words(CHUNK_SIZE + 1);
    let chunks = chunk_text(&text);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].split_whitespace().count(), CHUNK_SIZE);
    // second chunk covers the overlap tail + the one extra word
    assert_eq!(chunks[1].split_whitespace().count(), CHUNK_OVERLAP + 1);
}

#[test]
fn overlap_content_matches_between_chunks() {
    let text = words(CHUNK_SIZE + 1);
    let word_list: Vec<&str> = text.split_whitespace().collect();
    let chunks = chunk_text(&text);
    assert_eq!(chunks.len(), 2);

    // The last CHUNK_OVERLAP words of chunk 0 should equal the first CHUNK_OVERLAP words of chunk 1.
    let tail: Vec<&str> = word_list[CHUNK_SIZE - CHUNK_OVERLAP..CHUNK_SIZE].to_vec();
    let chunk1_words: Vec<&str> = chunks[1].split_whitespace().collect();
    let head: Vec<&str> = chunk1_words[..CHUNK_OVERLAP].to_vec();
    assert_eq!(tail, head);
}

#[test]
fn multiple_chunks_word_counts_are_correct() {
    // 3 full chunks worth of words to exercise the loop thoroughly
    let total = CHUNK_SIZE * 3;
    let text = words(total);
    let chunks = chunk_text(&text);

    // Each non-final chunk should have CHUNK_SIZE words.
    for c in &chunks[..chunks.len() - 1] {
        assert_eq!(c.split_whitespace().count(), CHUNK_SIZE);
    }
    // The final chunk must be non-empty.
    assert!(!chunks.last().unwrap().is_empty());
}
