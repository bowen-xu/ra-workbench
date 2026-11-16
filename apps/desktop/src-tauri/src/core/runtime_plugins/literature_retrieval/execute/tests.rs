use super::*;
use crate::runtime_plugins::{RuntimePluginCatalog, native::package};
use crate::{
    runtime_events::RuntimeEvents,
    storage::{AppStorePaths, AppStorePorts, AppStoreSources},
};
use research_workbench_native_sdk::data::DataValue;
use std::fs;

struct Fixture {
    _package: tempfile::TempDir,
    _data: tempfile::TempDir,
    state: State,
}
fn fixture() -> Fixture {
    let (package_root, catalog, _) = package::tests::literature_fixture(b"#!/bin/sh\nexit 42\n");
    let directory = package_root.path().join("writing");
    let manifest = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"literature.concept-retrieval","version":1}));
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    let mut copied = 0;
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let file = entry.file_name();
        if file.to_str().unwrap().starts_with("concept-retrieval-") {
            fs::copy(entry.path(), directory.join("data").join(file)).unwrap();
            copied += 1;
        }
    }
    assert_eq!(copied, 19);
    let catalog: Arc<RuntimePluginCatalog> = catalog;
    let selected = catalog.discover().unwrap().packages.remove(0);
    let accepted = package::accept_concept_retrieval(&catalog, &selected.package_key).unwrap();
    let resources = Arc::new(RetrievalResources::load(&catalog, &accepted).unwrap());
    let data = tempfile::tempdir().unwrap();
    let store = AppStore::open_with_paths(
        AppStorePaths {
            data_directory: data.path().into(),
            agent_configuration_directory: data.path().join("configuration"),
        },
        AppStorePorts::with_events(
            RuntimeEvents::new(|_, _| Ok(())),
            AppStoreSources::default(),
        ),
    )
    .unwrap();
    store.connection().unwrap().execute_batch("insert into concepts(id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,created_at,updated_at) values('first','First','','','','','','human_original',1,'now','now'),('second','Second','','','','','','human_original',1,'now','now');").unwrap();
    Fixture {
        _package: package_root,
        _data: data,
        state: State {
            store,
            resources,
            reads: ReadSet::default(),
            trace: "trace".into(),
            paper: None,
            query: "query".into(),
            fts: "query".into(),
            query_sha256: "a".repeat(64),
            lease_release_confirmed: None,
            trace_write_started: false,
        },
    }
}
fn reply() -> Reply {
    (
        Decision::Rejected {
            message: "native refusal".into(),
        },
        crate::ai::structured_resource_validation::DecisionOrigin {
            package_key: "a".repeat(64),
            instance_id: Uuid::new_v4().to_string(),
            native_request_id: Uuid::new_v4().to_string(),
        },
    )
}

#[test]
fn complete_prefix_rechecks_changed_and_previously_missing_rows_before_native_rejection() {
    for sql in [
        "update concepts set label='Changed' where id='first'",
        "insert into concepts(id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,created_at,updated_at) values('missing','Appeared','','','','','','human_original',1,'now','now')",
    ] {
        let mut f = fixture();
        let db = f.state.store.connection().unwrap();
        for id in ["first", "missing"] {
            f.state
                .resources
                .capture(
                    &db,
                    &mut f.state.reads,
                    "concept-identity",
                    vec![DataValue::Text(id.into())],
                )
                .unwrap();
        }
        assert!(matches!(
            consume(&db, &f.state, reply()),
            Err(Error::Rejected { .. })
        ));
        db.execute_batch(sql).unwrap();
        let error = consume(&db, &f.state, reply()).unwrap_err();
        assert!(
            matches!(&error, Error::Data(error) if error.code == "plugin_data_snapshot_changed")
        );
        let wire = error.into_wire(
            Some("client"),
            Some("trace"),
            "read",
            HbsOutcome::NotStarted,
        );
        assert_eq!(wire["nativeFailure"]["kind"], "sourceData");
        assert!(wire["nativeFailure"].get("nativeRequestId").is_none());
        assert!(db.is_autocommit());
    }
}

fn seed_trace(f: &Fixture) {
    f.state.store.connection().unwrap().execute(
        "insert into concept_retrieval_traces(id,query_sha256,status,config_json,context_text,context_sha256,seed_count,edge_count,source_count,external_paper_count,created_at,expires_at) values('trace',?1,'previewed','{\"semanticStatus\":\"used\"}','context',?1,2,0,0,0,'now','future')", ["a".repeat(64)],
    ).unwrap();
}

#[test]
fn stored_float_decoding_preserves_original_f32_f64_and_null_behavior() {
    let f = fixture();
    seed_trace(&f);
    let db = f.state.store.connection().unwrap();
    db.execute_batch("pragma ignore_check_constraints=on;")
        .unwrap();
    for (distance, score) in [
        (None, 1.0),
        (Some(-0.0), f64::MIN_POSITIVE),
        (Some(f64::INFINITY), f64::INFINITY),
        (Some(0.123456789), 0.123456789),
    ] {
        db.execute("delete from concept_retrieval_trace_seeds", [])
            .unwrap();
        db.execute("insert into concept_retrieval_trace_seeds(trace_id,concept_id,concept_revision,label,preferred_term,matched_by_json,ordinal,lexical_rank,vector_distance,rrf_score) values('trace','first',1,'First','','[\"lexical\"]',0,1,?1,?2)",params![distance,score]).unwrap();
        let actual = super::super::trace_read::read(&db, &f.state.resources, "trace").unwrap();
        assert_eq!(
            actual.seeds[0].vector_distance.map(f32::to_bits),
            distance.map(|value| (value as f32).to_bits())
        );
        assert_eq!(actual.seeds[0].rrf_score.to_bits(), score.to_bits());
    }
    assert!(db.is_autocommit());
}

#[test]
fn stored_first_row_domain_error_precedes_later_row_sql_conversion_error() {
    let f = fixture();
    seed_trace(&f);
    let db = f.state.store.connection().unwrap();
    db.execute_batch("insert into concept_retrieval_trace_seeds(trace_id,concept_id,concept_revision,label,preferred_term,matched_by_json,ordinal,lexical_rank,rrf_score) values('trace','first',1,'First','','[]',0,1,1.0),('trace','second',1,x'01','','[\"lexical\"]',1,1,1.0);").unwrap();
    let expected = E::InvalidInput(
        research_workbench_native_sdk::concept_retrieval::decode_trace_match_kinds("[]")
            .unwrap_err()
            .into(),
    );
    let actual = super::super::trace_read::read(&db, &f.state.resources, "trace").unwrap_err();
    assert!(matches!(expected, E::InvalidInput(_)));
    let wire = actual.into_wire(None, Some("trace"), "read", HbsOutcome::NotStarted);
    assert_eq!(wire["message"], expected.to_string());
    assert_eq!(wire["code"], "invalid_input");
    assert!(wire.get("nativeFailure").is_none());
}
