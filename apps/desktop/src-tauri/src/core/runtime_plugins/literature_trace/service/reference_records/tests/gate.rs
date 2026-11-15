//! Same finite FIFO handshake as the existing DOI/undo process fixtures.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};
use tokio::io::unix::AsyncFd;

pub(super) struct Gate {
    entered: AsyncFd<File>,
    release: File,
}

impl Gate {
    pub(super) fn new(directory: &Path) -> Self {
        let mode = nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR;
        let entered = directory.join("entered");
        let release = directory.join("release");
        nix::unistd::mkfifo(&entered, mode).unwrap();
        nix::unistd::mkfifo(&release, mode).unwrap();
        let open = |path| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(nix::fcntl::OFlag::O_NONBLOCK.bits())
                .open(path)
                .unwrap()
        };
        Self {
            entered: AsyncFd::new(open(entered)).unwrap(),
            release: open(release),
        }
    }

    pub(super) async fn entered(&self) {
        loop {
            let mut ready = self.entered.readable().await.unwrap();
            match ready.try_io(|fd| {
                let mut input = fd.get_ref();
                let mut marker = [0];
                input.read_exact(&mut marker)?;
                assert_eq!(marker, [b'e']);
                Ok(())
            }) {
                Ok(result) => return result.unwrap(),
                Err(_) => continue,
            }
        }
    }

    pub(super) fn release(&self) {
        let mut output = &self.release;
        output.write_all(b"go\n").unwrap();
    }
}
