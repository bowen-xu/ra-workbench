use research_docling_runtime::*;
use std::fs;
fn manifest(root: &std::path::Path) -> ModelManifest {
    let mut files = Vec::new();
    for name in REQUIRED_FILES.iter().copied().chain(["licenses/NOTICE"]) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"fixture").unwrap();
        let (size_bytes, sha256) = hash_file(&path).unwrap();
        files.push(ModelFile {
            path: name.into(),
            sha256,
            size_bytes,
        });
    }
    ModelManifest {
        schema_version: 1,
        engine_version: ENGINE_VERSION.into(),
        engine_commit: ENGINE_COMMIT.into(),
        precision: "fp32".into(),
        version: "test-1".into(),
        files,
    }
}
#[test]
fn identity_tracks_content_not_version_or_file_order() {
    let root = tempfile::tempdir().unwrap();
    let mut m = manifest(root.path());
    m.verify(root.path()).unwrap();
    let identity = m.identity().unwrap();
    m.version = "test-2".into();
    m.files.reverse();
    assert_eq!(identity, m.identity().unwrap());
    m.files[0].sha256 = "a".repeat(64);
    assert_ne!(identity, m.identity().unwrap());
}
#[test]
fn parser_revision_fences_services_and_parse_cache_without_changing_model_storage() {
    let root = tempfile::tempdir().unwrap();
    let m = manifest(root.path());
    let model = m.identity().unwrap();
    let execution = m.execution_identity().unwrap();
    assert_ne!(execution, model);
    assert!(!execution.contains("service="));
    assert_eq!(execution, format!("{model}:parser={PARSER_REVISION}"));
    assert_ne!(
        m.instance_directory_name().unwrap(),
        m.directory_name().unwrap()
    );
    assert_eq!(
        m.instance_directory_name().unwrap(),
        format!(
            "{}-parser-{PARSER_REVISION}-service-{SERVICE_REVISION}",
            m.directory_name().unwrap()
        )
    );
    assert!(m.verify(root.path()).is_ok());
}
#[test]
fn rejects_wrong_precision_missing_resources_and_paths() {
    let root = tempfile::tempdir().unwrap();
    let original = manifest(root.path());
    let mut m = original.clone();
    m.precision = "int8".into();
    assert!(m.validate().is_err());
    let mut m = original.clone();
    m.files.pop();
    assert!(m.validate().is_err());
    let mut m = original.clone();
    m.files[0].path = "../escape".into();
    assert!(m.validate().is_err());
    let mut m = original.clone();
    m.files.push(m.files[0].clone());
    assert!(m.validate().is_err());
    fs::write(root.path().join(REQUIRED_FILES[0]), b"corrupt").unwrap();
    assert!(original.verify(root.path()).is_err());
}
#[test]
fn model_identity_matches_publication_tool() {
    let root = tempfile::tempdir().unwrap();
    let m = manifest(root.path());
    let output=std::process::Command::new("node").args(["--input-type=module","-e",
        "import {modelBundleName} from './scripts/docling-models.mjs'; console.log(modelBundleName(JSON.parse(process.argv[1])))",&serde_json::to_string(&m.files).unwrap()])
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        m.directory_name().unwrap(),
        String::from_utf8(output.stdout).unwrap().trim()
    );
}
