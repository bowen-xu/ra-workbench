use super::*;

#[test]
fn reads_exact_text_versions_and_distinguishes_absent_empty_and_binary() {
    let temp = tempfile::tempdir().unwrap();
    let lease = store(temp.path()).create(manifest(), input()).unwrap();
    let work = lease.working_directory().unwrap();
    fs::write(work.join("sections/main.tex"), b"new text").unwrap();
    let change = lease.changes().unwrap().remove(0);
    let content = lease.change_content(&change).unwrap();
    assert_eq!(
        content.before,
        Some(ReviewContent::Text("user buffer".into()))
    );
    assert_eq!(content.after, Some(ReviewContent::Text("new text".into())));
    fs::write(work.join("sections/main.tex"), b"later text").unwrap();
    assert_eq!(lease.change_content(&change), Err(LeaseError::Conflict));
    fs::write(work.join("empty.tex"), b"").unwrap();
    fs::write(work.join("binary.dat"), [0xff, 0]).unwrap();
    for change in lease.changes().unwrap() {
        let content = lease.change_content(&change).unwrap();
        match change.path.as_str() {
            "empty.tex" => {
                assert_eq!(content.before, None);
                assert_eq!(content.after, Some(ReviewContent::Text(String::new())));
            }
            "binary.dat" => {
                assert_eq!(content.before, None);
                assert_eq!(content.after, Some(ReviewContent::Binary));
            }
            _ => {}
        }
    }
    fs::remove_file(work.join("sections/main.tex")).unwrap();
    let change = lease
        .changes()
        .unwrap()
        .into_iter()
        .find(|change| change.path == "sections/main.tex")
        .unwrap();
    assert_eq!(lease.change_content(&change).unwrap().after, None);
}

#[test]
fn refuses_links_and_forged_inventory_and_reports_oversized_text_without_truncation() {
    let temp = tempfile::tempdir().unwrap();
    let lease = store(temp.path()).create(manifest(), input()).unwrap();
    let work = lease.working_directory().unwrap();
    fs::write(work.join("large.tex"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    let mut change = lease.changes().unwrap().remove(0);
    assert_eq!(
        lease.change_content(&change).unwrap().after,
        Some(ReviewContent::TooLarge)
    );
    change.path = "../outside".into();
    assert_eq!(lease.change_content(&change), Err(LeaseError::Conflict));
    fs::remove_file(work.join("large.tex")).unwrap();
    symlink(temp.path().join("outside"), work.join("large.tex")).unwrap();
    assert!(lease.change_content(&change).is_err());
}
