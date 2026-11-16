//! Canned protocol responses only; Core owns the real SQL/log/Note fixture.
use super::*;
use crate::runtime_plugins::{
    literature_duplicates::tests::fixture::Fixture as CoreFixture,
    native::{
        package::{stage_for, tests::literature_fixture},
        transport::NativeHost,
    },
};
use research_workbench_native_sdk::{
    catalog::CatalogOutput,
    duplicate_review::{
        CAPABILITY_ID, CAPABILITY_VERSION, DuplicateCandidateBitsV1, DuplicateDecisionCompletionV1,
        DuplicateDecisionPreparationV1, DuplicateDecisionRequestV1, DuplicateRejectionCodeV1,
        DuplicateRejectionV1, F64BitsV1,
    },
};
use serde_json::Value;
use std::{fs, path::PathBuf};

const HOST: &str = r#"#!/bin/sh
record() { printf '%s\n' "$1" >> '__CALL_LOG__'; }
IFS= read -r line
record initialize
identity=${line%\"body\":*}
printf '%s"body":{"kind":"initialized","capability":"literature.note-trace-decision","version":1}}\n' "$identity"
reply() {
identity=${line%\"body\":*}
package=${line#*\"packageKey\":\"}; package=${package%%\"*}
instance=${line#*\"instanceId\":\"}; instance=${instance%%\"*}
request=${line#*\"requestId\":\"}; request=${request%%\"*}
result=catalog-result-${request}.json
printf '%s' "$1" > "$result"
bytes=$(/usr/bin/wc -c < "$result")
digest=$(/usr/bin/shasum -a 256 "$result"); digest=${digest%% *}
printf '%s"body":{"kind":"catalogResult","payload":{"relativeHandle":"%s","byteSize":%s,"sha256":"%s","packageKey":"%s","instanceId":"%s","requestId":"%s"}}}\n' "$identity" "$result" "$bytes" "$digest" "$package" "$instance" "$request"
}
IFS= read -r line
__EARLY_SHUTDOWN__
case "$line" in *'"operation":"__FIRST_OP__"'*) ;; *) exit 42 ;; esac
record __FIRST_LOG__
reply '__FIRST_REPLY__'
if [ '__STOP_FIRST__' = yes ]; then
# Keep only this fixture's owned root alive for deterministic retirement. This
# does not modify or explain the historical EOF/PermissionDenied fixture.
kill -STOP "$$"
exit 44
fi
IFS= read -r line || exit 43
case "$line" in
*'"operation":"finishDuplicateDecision"'*)
record finish
reply '__FINISH_REPLY__'
if [ '__STOP_FINISH__' = yes ]; then kill -STOP "$$"; exit 44; fi
IFS= read -r line || exit 43
;;
esac
case "$line" in *'"kind":"shutdown"'*) ;; *) exit 43 ;; esac
record shutdown
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
"#;

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Rejected,
    ChangedReady,
    Applied,
    WrongFinish,
    TamperedCandidates,
    History,
}

pub(super) fn request() -> DuplicateDecisionRequestV1 {
    DuplicateDecisionRequestV1 {
        candidate_id: "candidate".into(),
        decision: "dismiss".into(),
    }
}

pub(super) struct Fixture {
    pub data: CoreFixture,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    _package: tempfile::TempDir,
    _observations: tempfile::TempDir,
    calls: PathBuf,
    input_directory: PathBuf,
    execution_directory: PathBuf,
}

impl Fixture {
    pub async fn new(mode: Mode) -> Self {
        let data = CoreFixture::new();
        data.candidate("candidate", 0.72);
        data.db
            .lock()
            .unwrap()
            .execute_batch(
                "create table synthetic_service_decision_observations(value text not null)",
            )
            .unwrap();
        let first = match mode {
            Mode::History => CatalogOutput::MergeOperations { items: vec![] },
            Mode::Rejected => CatalogOutput::DuplicateDecisionPreparation {
                outcome: DuplicateDecisionPreparationV1::Rejected {
                    error: DuplicateRejectionV1 {
                        code: DuplicateRejectionCodeV1::InvalidInput,
                        message: "不支持的文献类型：DISMISS".into(),
                    },
                },
            },
            Mode::TamperedCandidates => CatalogOutput::DuplicateCandidates {
                items: vec![DuplicateCandidateBitsV1 {
                    id: "candidate".into(),
                    left_item_id: "survivor".into(),
                    left_title: "tampered title".into(),
                    right_item_id: "source".into(),
                    right_title: "Synthetic source".into(),
                    reason: "doi".into(),
                    confidence_bits: F64BitsV1::from_f64(0.72),
                }],
            },
            _ => {
                let mut prepared = request();
                if matches!(mode, Mode::ChangedReady) {
                    prepared.candidate_id = "swapped-candidate".into();
                }
                CatalogOutput::DuplicateDecisionPreparation {
                    outcome: DuplicateDecisionPreparationV1::Ready { request: prepared },
                }
            }
        };
        let finish = if matches!(mode, Mode::WrongFinish) {
            CatalogOutput::MergeOperations { items: vec![] }
        } else {
            CatalogOutput::DuplicateDecisionCompletion {
                outcome: DuplicateDecisionCompletionV1::Applied {},
            }
        };
        let observations = tempfile::tempdir().unwrap();
        let calls = observations.path().join("calls");
        let calls_text = calls.to_str().unwrap();
        assert!(!calls_text.contains('\''));
        let mut script = HOST
            .replace("__CALL_LOG__", calls_text)
            .replace(
                "__EARLY_SHUTDOWN__",
                if matches!(mode, Mode::History) {
                    r#"case "$line" in *'"kind":"shutdown"'*)
record shutdown
identity=${line%\"body\":*}
printf '%s"body":{"kind":"shutdownComplete"}}\n' "$identity"
exit 0
;; esac"#
                } else {
                    ""
                },
            )
            .replace(
                "__FIRST_OP__",
                if matches!(mode, Mode::History) {
                    "projectMergeOperations"
                } else if matches!(mode, Mode::TamperedCandidates) {
                    "projectDuplicateCandidates"
                } else {
                    "prepareDuplicateDecision"
                },
            )
            .replace(
                "__FIRST_LOG__",
                if matches!(mode, Mode::History) {
                    "history"
                } else if matches!(mode, Mode::TamperedCandidates) {
                    "candidates"
                } else {
                    "prepare"
                },
            )
            .replace(
                "__STOP_FIRST__",
                if matches!(mode, Mode::ChangedReady | Mode::TamperedCandidates) {
                    "yes"
                } else {
                    "no"
                },
            )
            .replace(
                "__STOP_FINISH__",
                if matches!(mode, Mode::WrongFinish) {
                    "yes"
                } else {
                    "no"
                },
            );
        for (marker, output) in [("__FIRST_REPLY__", first), ("__FINISH_REPLY__", finish)] {
            let json = serde_json::to_string(&output).unwrap();
            assert!(!json.contains('\''), "fixed synthetic shell literal");
            script = script.replace(marker, &json);
        }
        let (package, catalog, _) = literature_fixture(script.as_bytes());
        let root = package.path().join("writing");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id":CAPABILITY_ID,"version":CAPABILITY_VERSION}));
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for name in [
            "duplicate-candidates-list.v1.sql",
            "duplicate-candidate-decide.v1.sql",
            "duplicate-merge-history.v1.sql",
        ] {
            fs::copy(source.join(name), root.join("data").join(name)).unwrap();
        }
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        let staged = stage_for(&catalog, &key, NativeProfile::NoteTraceDecision).unwrap();
        let input_directory = staged.input_directory.as_ref().unwrap().path().to_owned();
        let execution_directory = staged.directory.path().to_owned();
        let mut host =
            NativeHost::spawn_for(staged, &key, NativeProfile::NoteTraceDecision).unwrap();
        timeout(WAIT, host.initialize()).await.unwrap().unwrap();
        let service = Arc::new(LiteratureTraceService::new(catalog));
        *service.host.lock().await = Some(host);
        Self {
            data,
            service,
            key,
            _package: package,
            _observations: observations,
            calls,
            input_directory,
            execution_directory,
        }
    }

    pub fn steps(&self) -> Vec<String> {
        fs::read_to_string(&self.calls)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    pub fn observation(&self) -> (Option<String>, bool, i64) {
        observation(&self.data)
    }

    pub async fn assert_cleaned(&self) {
        assert!(self.service.host.lock().await.is_none());
        assert!(!self.service.cleanup_failed.load(Ordering::Acquire));
        assert!(!self.input_directory.exists());
        assert!(!self.execution_directory.exists());
    }

    pub fn cold(self) -> CoreFixture {
        let Self { data, service, .. } = self;
        assert_eq!(Arc::strong_count(&service), 1);
        assert_eq!(Arc::strong_count(&data.db), 1);
        drop(service);
        data.reopen()
    }

    pub fn release_fenced(self) {
        let Self { data, service, .. } = self;
        assert_eq!(Arc::strong_count(&service), 1);
        assert_eq!(Arc::strong_count(&data.db), 1);
        assert!(data.db.lock().is_err());
        drop(service);
        // The recovery fence forbids SQL/shutdown. Close the final DB owner
        // before the reused Core fixture's directory is dropped.
        drop(data.db);
    }
}

pub(super) fn observation(data: &CoreFixture) -> (Option<String>, bool, i64) {
    data.db
        .lock()
        .unwrap()
        .query_row(
            "select (select decision from duplicate_candidates where id='candidate'),
            (select decided_at is not null from duplicate_candidates where id='candidate'),
            (select count(*) from synthetic_service_decision_observations)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}
