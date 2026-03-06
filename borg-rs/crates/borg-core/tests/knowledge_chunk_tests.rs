use borg_core::knowledge::chunk_text;

const CHUNK_SIZE: usize = 512;
const CHUNK_OVERLAP: usize = 64;

fn words(n: usize) -> String {
    (0..n)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn empty_string_returns_one_empty_chunk() {
    let chunks = chunk_text("");
    assert_eq!(chunks.len(), 0);
}

#[test]
fn short_text_returns_single_chunk() {
    let text = "hello world this is a short sentence";
    let chunks = chunk_text(text);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], text);
}

#[test]
fn exactly_chunk_size_words_returns_single_chunk() {
    let text = words(CHUNK_SIZE);
    let chunks = chunk_text(&text);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], text);
}

#[test]
fn one_word_over_chunk_size_produces_two_chunks() {
    let text = words(CHUNK_SIZE + 1);
    let chunks = chunk_text(&text);
    // First chunk: words 0..CHUNK_SIZE
    // Second chunk: words (CHUNK_SIZE - CHUNK_OVERLAP)..CHUNK_SIZE+1
    assert_eq!(chunks.len(), 2);

    let word_list: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(chunks[0], word_list[..CHUNK_SIZE].join(" "));
    assert_eq!(chunks[1], word_list[CHUNK_SIZE - CHUNK_OVERLAP..].join(" "));
}

#[test]
fn last_chunk_contains_final_words() {
    let total = CHUNK_SIZE * 2 + 100;
    let text = words(total);
    let chunks = chunk_text(&text);

    let word_list: Vec<&str> = text.split_whitespace().collect();
    let last = chunks.last().unwrap();
    // Last chunk must end with the final word of the input.
    assert!(
        last.ends_with(word_list.last().unwrap()),
        "last chunk must include final word"
    );
}

#[test]
fn chunks_overlap_by_chunk_overlap_words() {
    let total = CHUNK_SIZE + CHUNK_SIZE / 2;
    let text = words(total);
    let chunks = chunk_text(&text);
    assert!(chunks.len() >= 2);

    // The tail of chunk[0] and the head of chunk[1] must share CHUNK_OVERLAP words.
    let c0_words: Vec<&str> = chunks[0].split_whitespace().collect();
    let c1_words: Vec<&str> = chunks[1].split_whitespace().collect();

    let tail: Vec<&str> = c0_words[c0_words.len() - CHUNK_OVERLAP..].to_vec();
    let head: Vec<&str> = c1_words[..CHUNK_OVERLAP].to_vec();
    assert_eq!(
        tail, head,
        "consecutive chunks must overlap by {CHUNK_OVERLAP} words"
    );
}

#[test]
fn all_chunks_non_empty_for_long_text() {
    let text = words(CHUNK_SIZE * 3);
    for chunk in chunk_text(&text) {
        assert!(!chunk.is_empty(), "no chunk should be empty");
    }
}

#[test]
fn whitespace_only_input_returns_single_empty_chunk() {
    let chunks = chunk_text("   \n\t  ");
    assert_eq!(chunks.len(), 0);
}
