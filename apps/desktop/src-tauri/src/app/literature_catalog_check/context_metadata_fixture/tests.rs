use super::*;
#[test]
fn fixed_fresh_arguments_require_exact_key_and_never_accept_an_existing_store() {
    let key = "a".repeat(64);
    let valid = vec!["package-root".into(), FLAG.into(), key.clone()];
    let request = argument(&valid).unwrap().unwrap();
    assert_eq!(request.package_key, key);
    for invalid in [
        vec!["package-root".into(), FLAG.into()],
        vec!["package-root".into(), FLAG.into(), "A".repeat(64)],
        vec![
            "package-root".into(),
            FLAG.into(),
            key.clone(),
            "existing-store".into(),
        ],
        vec!["package-root".into(), format!("{FLAG}=existing-store"), key],
    ] {
        assert!(argument(&invalid).is_err());
    }
    assert!(
        argument(&["package-root".into(), "--metadata".into()])
            .unwrap()
            .is_none()
    );
}
