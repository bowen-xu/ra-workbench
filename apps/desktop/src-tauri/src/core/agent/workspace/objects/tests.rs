use super::*;
use std::fs;
use uuid::Uuid;
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("agent-proposal-objects-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn objects(&self) -> ProposalObjects {
        ProposalObjects::open(&self.0).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn version(bytes: &[u8]) -> FileVersion {
    FileVersion {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        byte_size: bytes.len() as u64,
        executable: true,
    }
}
#[test]
fn stages_empty_and_binary_bytes_and_reuses_identical_objects() {
    let f = Fixture::new();
    let store = f.objects();
    for bytes in [vec![], vec![0, 255, 0, 1], vec![42; 1024 * 1024 + 1]] {
        let v = version(&bytes);
        let path = store.stage(&v, &bytes).unwrap();
        let before = fs::metadata(f.0.join(&path)).unwrap().ino();
        assert_eq!(store.stage(&v, &bytes).unwrap(), path);
        assert_eq!(fs::metadata(f.0.join(&path)).unwrap().ino(), before);
        assert_eq!(fs::read(f.0.join(&path)).unwrap(), bytes);
        assert_eq!(fs::metadata(f.0.join(&path)).unwrap().mode() & 0o111, 0);
        store.verify_registered(&v, &path).unwrap();
    }
}
#[test]
fn refuses_wrong_or_corrupt_bytes_without_overwriting() {
    let f = Fixture::new();
    let store = f.objects();
    let v = version(b"original");
    assert_eq!(store.stage(&v, b"different"), Err(LeaseError::Conflict));
    assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    let path = store.stage(&v, b"original").unwrap();
    fs::write(f.0.join(&path), b"changed!").unwrap();
    assert_eq!(store.stage(&v, b"original"), Err(LeaseError::Conflict));
    assert_eq!(
        store.verify_registered(&v, &path),
        Err(LeaseError::Conflict)
    );
    assert_eq!(fs::read(f.0.join(path)).unwrap(), b"changed!");
}
#[test]
fn accepts_existing_pdf_path_but_rejects_escaping_and_linked_objects() {
    let f = Fixture::new();
    let store = f.objects();
    let bytes = b"%PDF-fixture";
    let v = version(bytes);
    let path = store.stage(&v, bytes).unwrap();
    let pdf = format!("{path}.pdf");
    fs::rename(f.0.join(&path), f.0.join(&pdf)).unwrap();
    store.verify_registered(&v, &pdf).unwrap();
    for invalid in [
        format!("../{pdf}"),
        format!("{path}../bad"),
        "short".into(),
        format!("{path}.pdf/extra"),
    ] {
        assert!(store.verify_registered(&v, &invalid).is_err());
    }
    let mut invalid = v.clone();
    invalid.sha256 = "x".into();
    assert!(store.verify_registered(&invalid, &pdf).is_err());
    std::os::unix::fs::symlink(f.0.join(&pdf), f.0.join(&path)).unwrap();
    assert!(store.stage(&v, bytes).is_err());
    fs::remove_file(f.0.join(&path)).unwrap();
    fs::hard_link(f.0.join(&pdf), f.0.join(&path)).unwrap();
    assert!(store.verify_registered(&v, &pdf).is_err());
    assert!(store.stage(&v, bytes).is_err());
}
#[test]
fn refuses_symlinked_hash_directories() {
    let f = Fixture::new();
    let outside = Fixture::new();
    let v = version(b"data");
    std::os::unix::fs::symlink(&outside.0, f.0.join(&v.sha256[..2])).unwrap();
    assert!(f.objects().stage(&v, b"data").is_err());
    assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);
}

#[test]
fn review_preserves_empty_text_and_reports_binary_or_large_content() {
    let f = Fixture::new();
    let objects = f.objects();
    for (bytes, expected) in [
        (vec![], super::super::ReviewContent::Text(String::new())),
        (vec![0, 255], super::super::ReviewContent::Binary),
        (
            vec![b'a'; 1024 * 1024 + 1],
            super::super::ReviewContent::TooLarge,
        ),
    ] {
        let version = version(&bytes);
        let path = objects.stage(&version, &bytes).unwrap();
        assert_eq!(
            objects.review_registered(&version, &path).unwrap(),
            expected
        );
    }
}
