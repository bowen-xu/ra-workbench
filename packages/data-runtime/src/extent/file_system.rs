//! Delegate all encoding, reads, writes, rotation and recovery to raft-engine.
use super::{Account, FileLease};
use crate::LogIoCounters;
use raft_engine::env::{FileSystem, Handle, Permission, WriteExt};
use std::{
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
};

pub(crate) struct ExtentFileSystem<F: FileSystem> {
    inner: Arc<F>,
    pub(crate) account: Arc<Account>,
    io: LogIoCounters,
}
pub(crate) struct ExtentHandle<H: Handle> {
    inner: Arc<H>,
    lease: Arc<FileLease>,
    io: LogIoCounters,
}
pub(crate) struct ExtentReader<R> {
    inner: R,
    _lease: Arc<FileLease>,
}
pub(crate) struct ExtentWriter<W> {
    inner: W,
    lease: Arc<FileLease>,
    offset: u64,
    io: LogIoCounters,
}
impl<F: FileSystem> ExtentFileSystem<F> {
    pub(crate) fn new(inner: Arc<F>, account: Arc<Account>, io: LogIoCounters) -> Self {
        Self { inner, account, io }
    }
}
impl<H: Handle> Handle for ExtentHandle<H> {
    fn truncate(&self, offset: usize) -> io::Result<()> {
        self.lease.grow(offset as u64)?;
        self.inner.truncate(offset)?;
        self.lease.truncate(offset as u64)
    }
    fn file_size(&self) -> io::Result<usize> {
        self.inner.file_size()
    }
    fn sync(&self) -> io::Result<()> {
        self.io.sync(|| self.inner.sync())
    }
}
impl<R: Read> Read for ExtentReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}
impl<R: Seek> Seek for ExtentReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}
impl<W: Write> Write for ExtentWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("log extent overflow"))?;
        // Charge the requested range before I/O; on a partial/error outcome keep
        // this conservative charge until confirmed truncate or owner reopen.
        self.lease.grow(end)?;
        let count = self.io.write(|| self.inner.write(bytes))?;
        self.offset += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl<W: Seek> Seek for ExtentWriter<W> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let offset = self.inner.seek(position)?;
        self.offset = offset;
        Ok(offset)
    }
}
impl<W: WriteExt + Seek> WriteExt for ExtentWriter<W> {
    fn truncate(&mut self, offset: usize) -> io::Result<()> {
        self.lease.grow(offset as u64)?;
        self.inner.truncate(offset)?;
        self.offset = self.inner.stream_position()?;
        self.lease.truncate(offset as u64)
    }
    fn allocate(&mut self, offset: usize, size: usize) -> io::Result<()> {
        let end = offset
            .checked_add(size)
            .ok_or_else(|| io::Error::other("log allocation extent overflow"))?;
        self.lease.grow(end as u64)?;
        self.inner.allocate(offset, size)
    }
}
impl<F: FileSystem> FileSystem for ExtentFileSystem<F> {
    type Handle = ExtentHandle<F::Handle>;
    type Reader = ExtentReader<F::Reader>;
    type Writer = ExtentWriter<F::Writer>;
    fn create<P: AsRef<Path>>(&self, path: P) -> io::Result<Self::Handle> {
        let inner = self.inner.create(&path)?;
        let lease = self
            .account
            .acquire(path.as_ref(), inner.file_size()? as u64)?;
        Ok(ExtentHandle {
            inner: Arc::new(inner),
            lease,
            io: self.io.clone(),
        })
    }
    fn open<P: AsRef<Path>>(&self, path: P, permission: Permission) -> io::Result<Self::Handle> {
        let inner = self.inner.open(&path, permission)?;
        let lease = self
            .account
            .acquire(path.as_ref(), inner.file_size()? as u64)?;
        Ok(ExtentHandle {
            inner: Arc::new(inner),
            lease,
            io: self.io.clone(),
        })
    }
    fn delete<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        self.inner.delete(&path)?;
        self.account.unlink(path.as_ref())
    }
    fn rename<P: AsRef<Path>>(&self, from: P, to: P) -> io::Result<()> {
        self.inner.rename(&from, &to)?;
        self.account.rename(from.as_ref(), to.as_ref())
    }
    fn new_reader(&self, handle: Arc<Self::Handle>) -> io::Result<Self::Reader> {
        Ok(ExtentReader {
            inner: self.inner.new_reader(handle.inner.clone())?,
            _lease: handle.lease.clone(),
        })
    }
    fn new_writer(&self, handle: Arc<Self::Handle>) -> io::Result<Self::Writer> {
        let mut inner = self.inner.new_writer(handle.inner.clone())?;
        let offset = inner.stream_position()?;
        Ok(ExtentWriter {
            inner,
            lease: handle.lease.clone(),
            offset,
            io: self.io.clone(),
        })
    }
    fn delete_metadata<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        self.inner.delete_metadata(path)
    }
    fn exists_metadata<P: AsRef<Path>>(&self, path: P) -> bool {
        self.inner.exists_metadata(path)
    }
}

#[cfg(test)]
mod io_tests;

#[cfg(test)]
mod tests {
    use super::*;
    struct PartialFailure(std::fs::File);
    impl Seek for PartialFailure {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.0.seek(position)
        }
    }
    impl Write for PartialFailure {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.write_all(&bytes[..3])?;
            Err(io::Error::other("synthetic partial write"))
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
    impl WriteExt for PartialFailure {
        fn truncate(&mut self, offset: usize) -> io::Result<()> {
            self.0.set_len(offset as u64)
        }
        fn allocate(&mut self, offset: usize, size: usize) -> io::Result<()> {
            self.0.set_len((offset + size / 2) as u64)?;
            Err(io::Error::other("synthetic partial allocation"))
        }
    }
    #[test]
    fn partial_io_failure_keeps_the_full_requested_extent_until_reopen() {
        for allocate in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("entry");
            let file = std::fs::File::create(&path).unwrap();
            let account = Account::inventory(dir.path(), 1024 * 1024, 128 * 1024).unwrap();
            let lease = account.acquire(&path, 0).unwrap();
            let mut writer = ExtentWriter {
                inner: PartialFailure(file),
                lease,
                offset: 0,
                io: LogIoCounters::default(),
            };
            let outcome = if allocate {
                writer.allocate(0, 512)
            } else {
                writer.write_all(&[1; 512])
            };
            assert!(outcome.is_err());
            assert_eq!(account.usage().unwrap().managed_file_extent_bytes, 512);
            drop(writer);
            drop(account);
            let reopened = Account::inventory(dir.path(), 1024 * 1024, 128 * 1024).unwrap();
            assert_eq!(
                reopened.usage().unwrap().managed_file_extent_bytes,
                if allocate { 256 } else { 3 }
            );
        }
    }
}
