use std::sync::Once;

static INIT: Once = Once::new();

fn init_modes() {
    INIT.call_once(|| {
        borg_core::modes::register_modes(borg_domains::all_modes());
    });
}

#[test]
fn test_swe_alias_resolves_same_as_sweborg() {
    init_modes();
    let by_alias = borg_core::modes::get_mode("swe");
    let by_canonical = borg_core::modes::get_mode("sweborg");
    assert!(by_alias.is_some(), "\"swe\" alias returned None");
    assert_eq!(by_alias.unwrap().name, by_canonical.unwrap().name);
}

#[test]
fn test_legal_alias_resolves_same_as_lawborg() {
    init_modes();
    let by_alias = borg_core::modes::get_mode("legal");
    let by_canonical = borg_core::modes::get_mode("lawborg");
    assert!(by_alias.is_some(), "\"legal\" alias returned None");
    assert_eq!(by_alias.unwrap().name, by_canonical.unwrap().name);
}

#[test]
fn test_canonical_name_roundtrips() {
    init_modes();
    let mode = borg_core::modes::get_mode("sweborg");
    assert!(mode.is_some(), "canonical name \"sweborg\" returned None");
    assert_eq!(mode.unwrap().name, "sweborg");
}

#[test]
fn test_unknown_name_returns_none() {
    init_modes();
    assert!(borg_core::modes::get_mode("notamode").is_none());
}
