use super::*;
use research_workbench_native_sdk::data::{DataAccess, DataField, DataType};
use serde_json::json;
use std::{fs, path::PathBuf};

const OWNER: &str = "writing_projects";
const RESOURCE: &str = "sql/registered.sql";
const SQL: &[u8] = b"select private_synthetic_value from synthetic_table where id=?1";

struct Fixture {
    _root: tempfile::TempDir,
    directory: PathBuf,
    catalog: RuntimePluginCatalog,
    package: RuntimePluginPackage,
}

impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("writing");
        fs::create_dir_all(directory.join("sql")).unwrap();
        fs::write(directory.join("index.js"), b"export {};").unwrap();
        fs::write(directory.join(RESOURCE), bytes).unwrap();
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&json!({
                "id":OWNER,"entry":"index.js","styles":[]
            }))
            .unwrap(),
        )
        .unwrap();
        let catalog = RuntimePluginCatalog::new(Some(root.path().into()));
        let package = catalog.discover().unwrap().packages.remove(0);
        Self {
            _root: root,
            directory,
            catalog,
            package,
        }
    }

    fn load(&self, grant: DataResourceGrant) -> Result<AcceptedDataResource, RuntimePluginError> {
        AcceptedDataResource::load(&self.catalog, &self.package, OWNER, RESOURCE, grant)
    }
}

fn grant(bytes: &[u8]) -> DataResourceGrant {
    DataResourceGrant {
        id: "writing.registered".into(),
        version: 1,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        access: DataAccess::Read,
        parameters: vec![DataField {
            data_type: DataType::Text,
            nullable: false,
        }],
        columns: vec![DataField {
            data_type: DataType::Integer,
            nullable: false,
        }],
    }
}

fn assert_error(error: RuntimePluginError, code: &str, key: &str, resource: &str) {
    assert_eq!(error.code, code);
    assert_eq!(error.package_key.as_deref(), Some(key));
    assert_eq!(error.resource.as_deref(), Some(resource));
    let wire = serde_json::to_string(&error).unwrap();
    assert!(!wire.contains("private_synthetic_value"));
    assert!(!wire.contains("synthetic_table"));
    assert!(!error.to_string().contains("select "));
}

#[test]
fn accepts_fixed_package_bytes_and_retains_an_immutable_clone() {
    let fixture = Fixture::new(SQL);
    let expected = grant(SQL);
    let accepted = fixture.load(expected.clone()).unwrap();
    let cloned = accepted.clone();
    fs::write(fixture.directory.join(RESOURCE), b"changed").unwrap();
    assert_eq!(accepted.grant(), &expected);
    assert_eq!(cloned.grant(), &expected);
    assert_eq!(accepted.sql().as_bytes(), SQL);
    assert_eq!(cloned.sql().as_bytes(), SQL);
    assert_eq!(accepted.owner_id(), OWNER);
    assert_eq!(accepted.package_key(), fixture.package.package_key);
    assert_error(
        fixture.load(expected).err().unwrap(),
        "runtime_plugin_resource_changed",
        &fixture.package.package_key,
        RESOURCE,
    );
}

#[test]
fn rejects_invalid_core_grants_before_reading_the_catalog() {
    let fixture = Fixture::new(SQL);
    let catalog = RuntimePluginCatalog::new(None);
    for invalid in [
        DataResourceGrant {
            version: 0,
            ..grant(SQL)
        },
        DataResourceGrant {
            id: "invalid/resource".into(),
            ..grant(SQL)
        },
        DataResourceGrant {
            sha256: "not-a-hash".into(),
            ..grant(SQL)
        },
        DataResourceGrant {
            columns: vec![],
            ..grant(SQL)
        },
    ] {
        assert_error(
            AcceptedDataResource::load(&catalog, &fixture.package, OWNER, RESOURCE, invalid)
                .err()
                .unwrap(),
            "runtime_plugin_data_grant_invalid",
            &fixture.package.package_key,
            RESOURCE,
        );
    }
}

#[test]
fn rejects_a_foreign_owner_even_when_the_digest_matches() {
    let fixture = Fixture::new(SQL);
    assert_error(
        AcceptedDataResource::load(
            &fixture.catalog,
            &fixture.package,
            "literature_library",
            RESOURCE,
            grant(SQL),
        )
        .err()
        .unwrap(),
        "runtime_plugin_data_owner_mismatch",
        &fixture.package.package_key,
        RESOURCE,
    );
}

#[test]
fn rejects_a_forged_package_manifest_and_a_changed_manifest_file() {
    let fixture = Fixture::new(SQL);
    let mut forged = fixture.package.clone();
    forged.manifest["version"] = "forged-version".into();
    assert_error(
        AcceptedDataResource::load(&fixture.catalog, &forged, OWNER, RESOURCE, grant(SQL))
            .err()
            .unwrap(),
        "runtime_plugin_data_manifest_mismatch",
        &fixture.package.package_key,
        RESOURCE,
    );
    fs::write(fixture.directory.join("manifest.json"), b"{changed}").unwrap();
    assert_error(
        fixture.load(grant(SQL)).err().unwrap(),
        "runtime_plugin_resource_changed",
        &fixture.package.package_key,
        "manifest.json",
    );
}

#[test]
fn rejects_a_package_not_discovered_by_the_supplied_catalog() {
    let fixture = Fixture::new(SQL);
    let foreign = Fixture::new(b"select a_different_value");
    assert_error(
        AcceptedDataResource::load(
            &foreign.catalog,
            &fixture.package,
            OWNER,
            RESOURCE,
            grant(SQL),
        )
        .err()
        .unwrap(),
        "runtime_plugin_package_unknown",
        &fixture.package.package_key,
        "manifest.json",
    );
}

#[test]
fn identical_discovered_content_keeps_the_catalogs_existing_root_independence() {
    let first = Fixture::new(SQL);
    let second = Fixture::new(SQL);
    assert_eq!(first.package.package_key, second.package.package_key);
    let accepted =
        AcceptedDataResource::load(&second.catalog, &first.package, OWNER, RESOURCE, grant(SQL))
            .unwrap();
    assert_eq!(accepted.sql().as_bytes(), SQL);
}

#[test]
fn rejects_a_grant_for_other_bytes_without_exposing_the_sql() {
    let fixture = Fixture::new(SQL);
    let wrong = DataResourceGrant {
        sha256: "0".repeat(64),
        ..grant(SQL)
    };
    assert_error(
        fixture.load(wrong).err().unwrap(),
        "runtime_plugin_data_digest_mismatch",
        &fixture.package.package_key,
        RESOURCE,
    );
}

#[test]
fn resource_paths_retain_the_catalogs_canonical_relative_path_policy() {
    let fixture = Fixture::new(SQL);
    for path in [
        "../registered.sql",
        "/registered.sql",
        "sql//registered.sql",
    ] {
        assert_error(
            AcceptedDataResource::load(&fixture.catalog, &fixture.package, OWNER, path, grant(SQL))
                .err()
                .unwrap(),
            "runtime_plugin_path_invalid",
            &fixture.package.package_key,
            path,
        );
    }
}

#[test]
fn rejects_non_utf8_empty_whitespace_and_nul_even_with_matching_grants() {
    for bytes in [b"".as_slice(), b" \r\n\t", b"select \0 secret", &[0xff]] {
        let fixture = Fixture::new(bytes);
        assert_error(
            fixture.load(grant(bytes)).err().unwrap(),
            "runtime_plugin_data_sql_invalid",
            &fixture.package.package_key,
            RESOURCE,
        );
    }
}

#[test]
fn accepts_text_without_parsing_or_executing_sql() {
    let bytes = b"not valid sql syntax, accepted as the granted resource bytes";
    let fixture = Fixture::new(bytes);
    assert_eq!(fixture.load(grant(bytes)).unwrap().sql().as_bytes(), bytes);
}

#[cfg(unix)]
#[test]
fn a_resource_symlink_cannot_replace_the_discovered_regular_file() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(SQL);
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), SQL).unwrap();
    fs::remove_file(fixture.directory.join(RESOURCE)).unwrap();
    symlink(outside.path(), fixture.directory.join(RESOURCE)).unwrap();
    assert_error(
        fixture.load(grant(SQL)).err().unwrap(),
        "runtime_plugin_resource_unsupported",
        &fixture.package.package_key,
        RESOURCE,
    );
}
