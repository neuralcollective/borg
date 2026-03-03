use borg_core::sandbox::SandboxMode;

#[test]
fn bwrap_parses_to_bwrap() {
    assert_eq!(SandboxMode::from_str_or_auto("bwrap"), Some(SandboxMode::Bwrap));
}

#[test]
fn docker_parses_to_docker() {
    assert_eq!(SandboxMode::from_str_or_auto("docker"), Some(SandboxMode::Docker));
}

#[test]
fn direct_parses_to_direct() {
    assert_eq!(SandboxMode::from_str_or_auto("direct"), Some(SandboxMode::Direct));
}

#[test]
fn none_parses_to_direct() {
    assert_eq!(SandboxMode::from_str_or_auto("none"), Some(SandboxMode::Direct));
}

#[test]
fn auto_returns_none() {
    assert_eq!(SandboxMode::from_str_or_auto("auto"), None);
}

#[test]
fn unknown_string_returns_none() {
    assert_eq!(SandboxMode::from_str_or_auto("podman"), None);
}

#[test]
fn empty_string_returns_none() {
    assert_eq!(SandboxMode::from_str_or_auto(""), None);
}

#[test]
fn case_insensitive_bwrap() {
    assert_eq!(SandboxMode::from_str_or_auto("BWRAP"), Some(SandboxMode::Bwrap));
    assert_eq!(SandboxMode::from_str_or_auto("Bwrap"), Some(SandboxMode::Bwrap));
}

#[test]
fn case_insensitive_docker() {
    assert_eq!(SandboxMode::from_str_or_auto("DOCKER"), Some(SandboxMode::Docker));
    assert_eq!(SandboxMode::from_str_or_auto("Docker"), Some(SandboxMode::Docker));
}

#[test]
fn case_insensitive_direct() {
    assert_eq!(SandboxMode::from_str_or_auto("DIRECT"), Some(SandboxMode::Direct));
    assert_eq!(SandboxMode::from_str_or_auto("NONE"), Some(SandboxMode::Direct));
}
