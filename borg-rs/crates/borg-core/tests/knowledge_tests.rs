use borg_core::knowledge::cosine_similarity;

#[test]
fn empty_slices_return_zero() {
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
}

#[test]
fn mismatched_lengths_return_zero() {
    assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
}

#[test]
fn all_zero_vectors_return_zero() {
    assert_eq!(cosine_similarity(&[0.0, 0.0, 0.0], &[0.0, 0.0, 0.0]), 0.0);
}

#[test]
fn one_zero_vector_returns_zero() {
    assert_eq!(cosine_similarity(&[1.0, 2.0, 3.0], &[0.0, 0.0, 0.0]), 0.0);
}

#[test]
fn identical_vectors_return_one() {
    let v = vec![1.0f32, 2.0, 3.0];
    let sim = cosine_similarity(&v, &v);
    assert!((sim - 1.0).abs() < 1e-6, "expected ~1.0, got {sim}");
}

#[test]
fn orthogonal_vectors_return_zero() {
    let sim = cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]);
    assert!(sim.abs() < 1e-6, "expected ~0.0, got {sim}");
}

#[test]
fn opposite_vectors_return_negative_one() {
    let sim = cosine_similarity(&[1.0, 2.0, 3.0], &[-1.0, -2.0, -3.0]);
    assert!((sim + 1.0).abs() < 1e-6, "expected ~-1.0, got {sim}");
}

#[test]
fn known_dot_product_value() {
    // a = [1, 0], b = [1, 1]/sqrt(2) => cos = 1/sqrt(2) ≈ 0.7071
    let a = [1.0f32, 0.0];
    let b = [1.0f32, 1.0];
    let sim = cosine_similarity(&a, &b);
    let expected = 1.0f32 / 2.0f32.sqrt();
    assert!((sim - expected).abs() < 1e-6, "expected {expected}, got {sim}");
}
