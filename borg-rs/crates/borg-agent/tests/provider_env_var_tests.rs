use borg_agent::provider_env_var;

#[test]
fn test_known_providers_map_correctly() {
    assert_eq!(provider_env_var("lexisnexis"), Some("LEXISNEXIS_API_KEY"));
    assert_eq!(provider_env_var("westlaw"), Some("WESTLAW_API_KEY"));
    assert_eq!(provider_env_var("clio"), Some("CLIO_API_KEY"));
    assert_eq!(provider_env_var("imanage"), Some("IMANAGE_API_KEY"));
    assert_eq!(provider_env_var("netdocuments"), Some("NETDOCUMENTS_API_KEY"));
    assert_eq!(provider_env_var("congress"), Some("CONGRESS_API_KEY"));
    assert_eq!(provider_env_var("openstates"), Some("OPENSTATES_API_KEY"));
    assert_eq!(provider_env_var("canlii"), Some("CANLII_API_KEY"));
    assert_eq!(provider_env_var("regulations_gov"), Some("REGULATIONS_GOV_API_KEY"));
}

#[test]
fn test_unknown_provider_returns_none() {
    assert_eq!(provider_env_var("unknown_provider"), None);
    assert_eq!(provider_env_var(""), None);
    assert_eq!(provider_env_var("LEXISNEXIS"), None); // case-sensitive
}
