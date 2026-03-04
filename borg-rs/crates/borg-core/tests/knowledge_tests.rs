use borg_core::knowledge::{bytes_to_embedding, cosine_similarity, embedding_to_bytes};

#[test]
fn identical_vectors_score_one() {
    let v = vec![1.0f32, 2.0, 3.0];
    let score = cosine_similarity(&v, &v);
    assert!((score - 1.0).abs() < 1e-6, "identical vectors: expected 1.0, got {score}");
}

#[test]
fn orthogonal_vectors_score_zero() {
    let a = vec![1.0f32, 0.0, 0.0];
    let b = vec![0.0f32, 1.0, 0.0];
    let score = cosine_similarity(&a, &b);
    assert!((score - 0.0).abs() < 1e-6, "orthogonal vectors: expected 0.0, got {score}");
}

#[test]
fn zero_norm_vector_returns_zero() {
    let zero = vec![0.0f32, 0.0, 0.0];
    let other = vec![1.0f32, 2.0, 3.0];
    assert_eq!(cosine_similarity(&zero, &other), 0.0);
    assert_eq!(cosine_similarity(&other, &zero), 0.0);
    assert_eq!(cosine_similarity(&zero, &zero), 0.0);
}

#[test]
fn known_numeric_vectors() {
    // [1,0] vs [1,1]/sqrt(2) → dot=1, |a|=1, |b|=sqrt(2) → similarity = 1/sqrt(2) ≈ 0.7071
    let a = vec![1.0f32, 0.0];
    let b = vec![1.0f32, 1.0];
    let expected = 1.0f32 / 2.0f32.sqrt();
    let score = cosine_similarity(&a, &b);
    assert!(
        (score - expected).abs() < 1e-5,
        "expected {expected}, got {score}"
    );
}

#[test]
fn mismatched_lengths_return_zero() {
    let a = vec![1.0f32, 2.0];
    let b = vec![1.0f32, 2.0, 3.0];
    assert_eq!(cosine_similarity(&a, &b), 0.0);
}

#[test]
fn empty_vectors_return_zero() {
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
}

fn roundtrip(vec: &[f32]) -> Vec<f32> {
    bytes_to_embedding(&embedding_to_bytes(vec))
}

fn bits_eq(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits()
}

#[test]
fn roundtrip_empty() {
    let out: Vec<f32> = roundtrip(&[]);
    assert!(out.is_empty());
}

#[test]
fn roundtrip_single() {
    let v = vec![1.5f32];
    let out = roundtrip(&v);
    assert_eq!(out.len(), 1);
    assert!(bits_eq(out[0], v[0]));
}

#[test]
fn roundtrip_384_elements() {
    let v: Vec<f32> = (0..384).map(|i| i as f32 * 0.001 + 0.5).collect();
    let out = roundtrip(&v);
    assert_eq!(out.len(), v.len());
    for (a, b) in v.iter().zip(out.iter()) {
        assert!(bits_eq(*a, *b), "mismatch: {} vs {}", a, b);
    }
}

#[test]
fn roundtrip_special_values() {
    let v = vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0f32, -0.0f32];
    let out = roundtrip(&v);
    assert_eq!(out.len(), v.len());
    for (a, b) in v.iter().zip(out.iter()) {
        assert!(bits_eq(*a, *b), "bit mismatch: {:?} vs {:?}", a.to_bits(), b.to_bits());
    }
}
