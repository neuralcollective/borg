use borg_core::sandbox::Sandbox;

#[test]
fn branch_hash_empty_string_is_8_hex_chars() {
    let h = Sandbox::branch_hash("");
    assert_eq!(h.len(), 8, "branch_hash must return exactly 8 chars, got {h:?}");
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()), "branch_hash must be lowercase hex, got {h:?}");
}

#[test]
fn branch_hash_is_stable() {
    assert_eq!(Sandbox::branch_hash("main"), Sandbox::branch_hash("main"));
    assert_eq!(Sandbox::branch_hash("feature/foo-123"), Sandbox::branch_hash("feature/foo-123"));
}

#[test]
fn branch_hash_output_length_always_8() {
    for name in &["", "a", "main", "feature/very-long-branch-name-that-exceeds-normal-length"] {
        let h = Sandbox::branch_hash(name);
        assert_eq!(h.len(), 8, "branch_hash({name:?}) must be 8 chars, got {h:?}");
    }
}

#[test]
fn branch_hash_different_inputs_differ() {
    assert_ne!(
        Sandbox::branch_hash("main"),
        Sandbox::branch_hash("feature/foo"),
        "different branch names should produce different hashes"
    );
}

#[test]
fn branch_hash_known_value() {
    // FNV-1a of "main": verify a known stable value so regressions are caught
    let h = Sandbox::branch_hash("main");
    assert_eq!(h, "1f5962a2", "FNV-1a hash of 'main' changed — breaking volume name stability");
}

#[test]
fn branch_volume_name_matches_pattern() {
    let name = Sandbox::branch_volume_name("myrepo", "feature/abc", "target");
    let h = Sandbox::branch_hash("feature/abc");
    assert_eq!(name, format!("borg-cache-myrepo-{h}-target"));
    // Verify the static parts
    assert!(name.starts_with("borg-cache-"), "must start with borg-cache-");
    assert!(name.ends_with("-target"), "must end with the cache_type");
    // The hash segment must be exactly 8 hex chars
    assert_eq!(h.len(), 8);
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn main_volume_name_uses_main_branch() {
    let main_vol = Sandbox::main_volume_name("myrepo", "target");
    let explicit = Sandbox::branch_volume_name("myrepo", "main", "target");
    assert_eq!(main_vol, explicit, "main_volume_name must equal branch_volume_name with 'main'");
}
