use super::*;
use std::{fs, path::Path};

fn fixture(
    latest: bool,
) -> (
    tempfile::TempDir,
    RuntimePluginCatalog,
    RuntimePluginPackage,
) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("literature");
    fs::create_dir_all(directory.join("data")).unwrap();
    fs::write(
        directory.join("manifest.json"),
        r#"{"id":"literature_library","entry":"entry.js","styles":[]}"#,
    )
    .unwrap();
    fs::write(directory.join("entry.js"), "export {};").unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for name in ["snapshot-hash", "snapshot-rows", "validation-latest"] {
        if name == "validation-latest" && !latest {
            continue;
        }
        let file = format!("reading-evidence-{name}.v1.sql");
        fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
    }
    let catalog = RuntimePluginCatalog::new(Some(root.path().to_owned()));
    let package = catalog.discover().unwrap().packages.remove(0);
    (root, catalog, package)
}

#[test]
fn original_two_read_loader_does_not_require_the_hbs_only_latest_file() {
    let (_root, catalog, package) = fixture(false);
    assert!(ReadingEvidenceResources::load(&catalog, &package).is_ok());
    assert!(HbsReadingEvidenceResources::load(&catalog, &package).is_err());
}

#[test]
fn validation_latest_keeps_created_at_then_id_order_and_exact_paper() {
    let (_root, catalog, package) = fixture(true);
    let reading = HbsReadingEvidenceResources::load(&catalog, &package).unwrap();
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table paper_context_snapshots(id text,paper_id text,created_at text);insert into paper_context_snapshots values('z','paper','2026-11-07'),('a','paper','2026-11-07'),('zz','paper','2025-01-01'),('other','another','2027-01-01');").unwrap();
    assert_eq!(
        reading
            .validation_latest_snapshot(&db, "paper")
            .unwrap()
            .as_deref(),
        Some("z")
    );
    assert_eq!(
        reading.validation_latest_snapshot(&db, " paper").unwrap(),
        None
    );
    assert_eq!(
        reading.validation_latest_snapshot(&db, "missing").unwrap(),
        None
    );
    assert_eq!(reading.package_key(), package.package_key);
}

#[test]
fn validation_latest_preserves_real_sql_and_codec_failure_as_safe_data() {
    let (_root, catalog, package) = fixture(true);
    let reading = HbsReadingEvidenceResources::load(&catalog, &package).unwrap();
    let db = Connection::open_in_memory().unwrap();
    let SourceReadFailure::Native(sql) = reading
        .validation_latest_snapshot(&db, "paper")
        .unwrap_err()
    else {
        panic!("native SQL cause");
    };
    assert_eq!(
        sql.resource_id,
        "literature.reading-evidence.validation-latest"
    );
    assert!(sql.sqlite_code.is_some());
    let serialized = serde_json::to_string(&sql).unwrap();
    assert!(!serialized.contains("select "));
    db.execute_batch("create table paper_context_snapshots(id,paper_id,created_at);insert into paper_context_snapshots values(x'00','paper','now');").unwrap();
    let SourceReadFailure::Native(codec) = reading
        .validation_latest_snapshot(&db, "paper")
        .unwrap_err()
    else {
        panic!("native codec cause");
    };
    assert_eq!(codec.resource_id, sql.resource_id);
    assert!(!codec.safe_cause.is_empty());
}
