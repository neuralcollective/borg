use borg_core::knowledge::cosine_similarity;

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
