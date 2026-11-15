use super::*;

#[test]
fn fixed_mode_rejects_extra_or_arbitrary_write_arguments() {
    let arguments = |values: &[&str]| {
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
    };
    let exact = arguments(&["/controlled/package", FLAG]);
    assert_eq!(
        package_argument(&exact).unwrap(),
        Some("/controlled/package")
    );
    for invalid in [
        vec!["/controlled/package", FLAG, "/arbitrary/store"],
        vec!["/controlled/package", FLAG, "--body=arbitrary text"],
        vec!["/controlled/package", FLAG, "--sql=delete from anything"],
        vec!["/controlled/package", FLAG, "--unknown-command"],
        vec![
            "/controlled/package",
            "--structured-hbs-fixture={\"body\":\"arbitrary\"}",
        ],
        vec![FLAG],
    ] {
        assert!(
            package_argument(&arguments(&invalid)).is_err(),
            "must reject before any store or service: {invalid:?}"
        );
    }
    for unchanged in [
        vec!["/controlled/package"],
        vec!["/controlled/package", "--save"],
        vec!["/controlled/package", "--attachments"],
    ] {
        assert!(package_argument(&arguments(&unchanged)).unwrap().is_none());
    }
}
