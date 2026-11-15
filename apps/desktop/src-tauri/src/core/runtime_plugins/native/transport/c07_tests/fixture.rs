//! Real staged shell/pipe ownership; only the first termination result is injected.
use super::*;
use crate::runtime_plugins::{
    RuntimePluginCatalog,
    native::package::{stage_for, tests::literature_fixture},
};
use nix::{sys::stat::Mode, unistd::mkfifo};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::Arc,
};

const SCRIPT: &str = r#"#!/bin/sh
IFS= read -r line || exit 41
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
IFS= read -r line || exit 42
printf '%s\n' "$line" > business-request
identity=${line%\"body\":*}
printf '%s"body":{"kind":"noteTraceDecision","decision":{"status":"rejected","reason":"not_found"}}}\n' "$identity"
__WAIT_FOR_EXIT__
printf 'synthetic final stderr\n' >&2
exit 0
"#;
const WAIT_FOR_STDIN: &str = r#"if IFS= read -r line; then
case "$line" in *'"kind":"shutdown"'*) ;; *) exit 43 ;; esac
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
IFS= read -r eof
fi"#;
const WAIT_FOR_FIFO: &str = "IFS= read -r release < release-exit";

pub(super) struct Fixture {
    pub host: NativeHost,
    pub directory: PathBuf,
    pub executable: PathBuf,
    gate: Option<File>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    _package: tempfile::TempDir,
}

impl Fixture {
    pub async fn new(hold_exit: bool) -> Self {
        let script = SCRIPT.replace(
            "__WAIT_FOR_EXIT__",
            if hold_exit {
                WAIT_FOR_FIFO
            } else {
                WAIT_FOR_STDIN
            },
        );
        let (package, catalog, key) = literature_fixture(script.as_bytes());
        let stage = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        let directory = stage.input_directory.as_ref().unwrap().path().to_owned();
        let executable = stage.directory.path().to_owned();
        let gate = hold_exit.then(|| {
            let path = directory.join("release-exit");
            mkfifo(&path, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
            // A single short token releases a real FIFO reader. RDWR avoids
            // a blocking open; there is no timer, polling or detached writer.
            OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(nix::libc::O_NONBLOCK)
                .open(path)
                .unwrap()
        });
        let mut host =
            NativeHost::spawn_for(stage, &key, NativeProfile::NoteTraceDecision).unwrap();
        exchange_once(&mut host).await;
        Self {
            host,
            directory,
            executable,
            gate,
            catalog,
            key,
            _package: package,
        }
    }

    pub fn identity(&self) -> (String, String) {
        self.host.request_identity().unwrap()
    }

    pub fn request_bytes(&self) -> Vec<u8> {
        fs::read(self.directory.join("business-request")).unwrap()
    }

    pub fn primary(&self) -> Box<NativeHostError> {
        self.host.error(
            "plugin_host_response_mismatch",
            "Synthetic original binding failure",
        )
    }

    pub fn inject(&mut self, operation: ProcessIoOperation) {
        assert!(self.host.initial_termination_error.is_none());
        self.host.initial_termination_error = Some(ProcessError::OperationIo {
            operation,
            kind: io::ErrorKind::PermissionDenied,
            raw_os_error: Some(nix::libc::EPERM),
        });
    }

    pub fn release_exit(&mut self) {
        self.gate.as_mut().unwrap().write_all(b"release\n").unwrap();
    }

    pub async fn assert_drained(&mut self) {
        // PipeProcess::None requires both actual EOFs, this Child's actual
        // reap and the original pending group settled. It is not a bool mock.
        assert!(
            timeout(WAIT, self.host.process.next_output())
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert_eq!(self.host.process.exit().unwrap().code, Some(0));
    }

    pub async fn replacement(&self) -> (NativeHost, PathBuf, PathBuf) {
        let stage = stage_for(&self.catalog, &self.key, NativeProfile::NoteTraceDecision).unwrap();
        let directory = stage.input_directory.as_ref().unwrap().path().to_owned();
        let executable = stage.directory.path().to_owned();
        let mut host =
            NativeHost::spawn_for(stage, &self.key, NativeProfile::NoteTraceDecision).unwrap();
        exchange_once(&mut host).await;
        (host, directory, executable)
    }

    pub fn obstruct_input_unlink(&mut self) -> PathBuf {
        // Stage a real lease under the last actual exchange identity. This is
        // an input-cleanup fixture, not another dispatched/replayed request.
        let request = literature::Request {
            protocol_version: literature::PROTOCOL_VERSION,
            plugin_id: literature::PLUGIN_ID.into(),
            package_key: self.host.package_key.clone(),
            instance_id: self.host.instance_id.clone(),
            request_id: self.host.last_request_id.clone().unwrap(),
            body: literature::RequestBody::Initialize,
        };
        let (lease, reference) = InputLease::stage(
            &self.directory,
            &request,
            &input(),
            Arc::clone(&self.host.input_cleanup),
        )
        .unwrap();
        assert!(self.host.input_lease.is_none());
        self.host.input_lease = Some(lease);
        let path = self.directory.join(reference.relative_handle);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        path
    }
}

pub(super) fn input() -> literature::SnapshotInput {
    literature::SnapshotInput {
        query: "Synthetic C07 query".into(),
        row: None,
    }
}

async fn exchange_once(host: &mut NativeHost) {
    timeout(WAIT, host.initialize()).await.unwrap().unwrap();
    let decision = timeout(WAIT, host.decide_note_trace(input(), WAIT))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        decision,
        literature::TraceDecision::Rejected {
            reason: research_workbench_native_sdk::retrieval_trace::TraceRejection::NotFound
        }
    ));
}
