use borg_core::knowledge::{chunk_text, cosine_similarity};

// ── chunk_text ────────────────────────────────────────────────────────────────

#[test]
fn test_chunk_text_short_returns_single_chunk() {
    let text = "hello world foo bar";
    let chunks = chunk_text(text);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], "hello world foo bar");
}

#[test]
fn test_chunk_text_empty_returns_single_empty_chunk() {
    let chunks = chunk_text("");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], "");
}

#[test]
fn test_chunk_text_whitespace_normalised_in_single_chunk() {
    let text = "  foo   bar  baz  ";
    let chunks = chunk_text(text);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], "foo bar baz");
}

#[test]
fn test_chunk_text_exactly_512_words_is_single_chunk() {
    let text: String = (0..512).map(|i| format!("word{i}")).collect::<Vec<_>>().join(" ");
    let chunks = chunk_text(&text);
    assert_eq!(chunks.len(), 1);
}

#[test]
fn test_chunk_text_513_words_produces_two_chunks() {
    let text: String = (0..513).map(|i| format!("word{i}")).collect::<Vec<_>>().join(" ");
    let chunks = chunk_text(&text);
    // First chunk: words[0..512], second chunk: words[512-64..513] = words[448..513]
    assert_eq!(chunks.len(), 2);
}

#[test]
fn test_chunk_text_long_text_correct_chunk_count() {
    // 1024 words → chunks:
    //   chunk 0: [0..512]
    //   chunk 1: [448..960]
    //   chunk 2: [896..1024]
    let words: Vec<String> = (0..1024).map(|i| format!("w{i}")).collect();
    let text = words.join(" ");
    let chunks = chunk_text(&text);
    assert_eq!(chunks.len(), 3);
}

#[test]
fn test_chunk_text_adjacent_chunks_overlap() {
    // 600 words: chunk 0 = words[0..512], chunk 1 = words[448..600]
    let words: Vec<String> = (0..600).map(|i| format!("w{i}")).collect();
    let text = words.join(" ");
    let chunks = chunk_text(&text);
    assert_eq!(chunks.len(), 2);

    // The last 64 words of chunk 0 must equal the first 64 words of chunk 1.
    let chunk0_words: Vec<&str> = chunks[0].split_whitespace().collect();
    let chunk1_words: Vec<&str> = chunks[1].split_whitespace().collect();

    let tail_of_chunk0 = &chunk0_words[chunk0_words.len() - 64..];
    let head_of_chunk1 = &chunk1_words[..64];
    assert_eq!(tail_of_chunk0, head_of_chunk1);
}

// ── cosine_similarity ─────────────────────────────────────────────────────────

#[test]
fn test_cosine_similarity_identical_vectors() {
    let v = vec![1.0f32, 2.0, 3.0];
    let result = cosine_similarity(&v, &v);
    assert!((result - 1.0).abs() < 1e-6, "identical vectors must give 1.0, got {result}");
}

#[test]
fn test_cosine_similarity_orthogonal_vectors() {
    let a = vec![1.0f32, 0.0];
    let b = vec![0.0f32, 1.0];
    let result = cosine_similarity(&a, &b);
    assert!(result.abs() < 1e-6, "orthogonal vectors must give 0.0, got {result}");
}

#[test]
fn test_cosine_similarity_zero_vector_returns_zero() {
    let a = vec![0.0f32, 0.0, 0.0];
    let b = vec![1.0f32, 2.0, 3.0];
    assert_eq!(cosine_similarity(&a, &b), 0.0);
    assert_eq!(cosine_similarity(&b, &a), 0.0);
    assert_eq!(cosine_similarity(&a, &a), 0.0);
}

#[test]
fn test_cosine_similarity_mismatched_lengths_returns_zero() {
    let a = vec![1.0f32, 2.0];
    let b = vec![1.0f32, 2.0, 3.0];
    assert_eq!(cosine_similarity(&a, &b), 0.0);
}

#[test]
fn test_cosine_similarity_empty_slices_returns_zero() {
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
}
