use super::*;

struct ShortWriter {
    file: std::fs::File,
    fail_after_partial: bool,
}
impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fail_after_partial {
            self.file.write_all(&bytes[..bytes.len().min(3)])?;
            return Err(io::Error::other("synthetic partial write error"));
        }
        self.file.write(&bytes[..bytes.len().min(2)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[test]
fn delegated_short_writes_count_returned_bytes_but_failed_partial_bytes_stay_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("short");
    let file = std::fs::File::create(&path).unwrap();
    let account = Account::inventory(directory.path(), 1024 * 1024, 128 * 1024).unwrap();
    let io = LogIoCounters::default();
    let mut writer = ExtentWriter {
        inner: ShortWriter {
            file,
            fail_after_partial: false,
        },
        lease: account.acquire(&path, 0).unwrap(),
        offset: 0,
        io: io.clone(),
    };
    writer.write_all(b"1234567").unwrap();
    assert_eq!(
        io.snapshot(),
        crate::LogIoSnapshot {
            write_calls: 4,
            write_bytes: 7,
            ..Default::default()
        }
    );
    assert_eq!(writer.write(b"").unwrap(), 0);
    writer.inner.fail_after_partial = true;
    assert!(writer.write(b"unknown partial").is_err());
    assert_eq!(
        io.snapshot(),
        crate::LogIoSnapshot {
            write_calls: 6,
            write_bytes: 7,
            write_errors: 1,
            ..Default::default()
        }
    );
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 10);
    // The wrapper's conservative extent is deliberately different from both
    // successful returned bytes and the actual partial file length.
    assert!(account.usage().unwrap().managed_file_extent_bytes > 10);
    let before = io.snapshot();
    assert!(writer.write(&vec![0; 2 * 1024 * 1024]).is_err());
    assert_eq!(
        io.snapshot(),
        before,
        "quota refusal never reached the delegate"
    );
}

#[test]
fn resize_and_allocation_are_not_reported_as_delegated_write_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let io = LogIoCounters::default();
    let fs = ExtentFileSystem::new(
        Arc::new(raft_engine::env::DefaultFileSystem),
        Account::inventory(directory.path(), 1024 * 1024, 128 * 1024).unwrap(),
        io.clone(),
    );
    let handle = Arc::new(fs.create(directory.path().join("extent")).unwrap());
    let mut writer = fs.new_writer(handle.clone()).unwrap();
    writer.allocate(0, 1024).unwrap();
    writer.truncate(512).unwrap();
    handle.truncate(256).unwrap();
    writer.flush().unwrap();
    assert_eq!(io.snapshot(), crate::LogIoSnapshot::default());
    assert_eq!(
        std::fs::metadata(directory.path().join("extent"))
            .unwrap()
            .len(),
        256
    );
}
