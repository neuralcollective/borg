/// Tests for the backlog scoring JSON field extraction pattern used in auto_triage.
///
/// The loop uses `req!(get_i64("field"))` — a local macro that skips the item
/// when the field is absent or the wrong type. These tests verify that behavior
/// by simulating the extraction logic directly against serde_json Values.

fn extract_triage_fields(item: &serde_json::Value) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let get_i64 = |k: &str| item.get(k).and_then(|v| v.as_i64());
    Some((
        get_i64("id")?,
        get_i64("impact")?,
        get_i64("feasibility")?,
        get_i64("risk")?,
        get_i64("effort")?,
        get_i64("score")?,
    ))
}

fn count_processable(items: &[serde_json::Value]) -> usize {
    items
        .iter()
        .filter(|item| extract_triage_fields(item).is_some())
        .count()
}

fn make_complete_item(id: i64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "impact": 8,
        "feasibility": 7,
        "risk": 3,
        "effort": 5,
        "score": 82,
        "reasoning": "High value",
        "dismiss": false
    })
}

#[test]
fn test_complete_item_is_processed() {
    let item = make_complete_item(42);
    let result = extract_triage_fields(&item);
    assert!(result.is_some());
    let (id, impact, feasibility, risk, effort, score) = result.unwrap();
    assert_eq!(id, 42);
    assert_eq!(impact, 8);
    assert_eq!(feasibility, 7);
    assert_eq!(risk, 3);
    assert_eq!(effort, 5);
    assert_eq!(score, 82);
}

#[test]
fn test_missing_id_skips_item() {
    let item = serde_json::json!({
        "impact": 8, "feasibility": 7, "risk": 3, "effort": 5, "score": 82
    });
    assert!(extract_triage_fields(&item).is_none());
}

#[test]
fn test_missing_impact_skips_item() {
    let item = serde_json::json!({
        "id": 1, "feasibility": 7, "risk": 3, "effort": 5, "score": 82
    });
    assert!(extract_triage_fields(&item).is_none());
}

#[test]
fn test_missing_feasibility_skips_item() {
    let item = serde_json::json!({
        "id": 1, "impact": 8, "risk": 3, "effort": 5, "score": 82
    });
    assert!(extract_triage_fields(&item).is_none());
}

#[test]
fn test_missing_risk_skips_item() {
    let item = serde_json::json!({
        "id": 1, "impact": 8, "feasibility": 7, "effort": 5, "score": 82
    });
    assert!(extract_triage_fields(&item).is_none());
}

#[test]
fn test_missing_effort_skips_item() {
    let item = serde_json::json!({
        "id": 1, "impact": 8, "feasibility": 7, "risk": 3, "score": 82
    });
    assert!(extract_triage_fields(&item).is_none());
}

#[test]
fn test_missing_score_skips_item() {
    let item = serde_json::json!({
        "id": 1, "impact": 8, "feasibility": 7, "risk": 3, "effort": 5
    });
    assert!(extract_triage_fields(&item).is_none());
}

#[test]
fn test_string_field_instead_of_integer_skips_item() {
    let item = serde_json::json!({
        "id": "not-a-number",
        "impact": 8, "feasibility": 7, "risk": 3, "effort": 5, "score": 82
    });
    assert!(extract_triage_fields(&item).is_none());
}

#[test]
fn test_float_field_skips_item() {
    // JSON floats are not returned by as_i64() unless they are whole numbers
    // representable as i64. A fractional value must be rejected.
    let item = serde_json::json!({
        "id": 1.5,
        "impact": 8, "feasibility": 7, "risk": 3, "effort": 5, "score": 82
    });
    assert!(extract_triage_fields(&item).is_none());
}

#[test]
fn test_empty_object_skips_item() {
    let item = serde_json::json!({});
    assert!(extract_triage_fields(&item).is_none());
}

#[test]
fn test_mixed_batch_only_counts_complete_items() {
    let items = vec![
        make_complete_item(1),
        serde_json::json!({ "id": 2, "impact": 5 }), // incomplete
        make_complete_item(3),
        serde_json::json!({}), // empty
        make_complete_item(5),
    ];
    assert_eq!(count_processable(&items), 3);
}

#[test]
fn test_all_items_complete() {
    let items: Vec<_> = (1..=5).map(make_complete_item).collect();
    assert_eq!(count_processable(&items), 5);
}

#[test]
fn test_no_items_complete() {
    let items = vec![
        serde_json::json!({ "id": 1 }),
        serde_json::json!({ "score": 50 }),
    ];
    assert_eq!(count_processable(&items), 0);
}

#[test]
fn test_negative_field_values_are_valid() {
    // Negative scores/impacts are unusual but valid i64s — must not be skipped.
    let item = serde_json::json!({
        "id": 99,
        "impact": -1,
        "feasibility": -2,
        "risk": -3,
        "effort": -4,
        "score": -10
    });
    let result = extract_triage_fields(&item);
    assert!(result.is_some());
    let (id, impact, _, _, _, score) = result.unwrap();
    assert_eq!(id, 99);
    assert_eq!(impact, -1);
    assert_eq!(score, -10);
}

#[test]
fn test_zero_values_are_valid() {
    let item = serde_json::json!({
        "id": 0,
        "impact": 0,
        "feasibility": 0,
        "risk": 0,
        "effort": 0,
        "score": 0
    });
    assert!(extract_triage_fields(&item).is_some());
}
