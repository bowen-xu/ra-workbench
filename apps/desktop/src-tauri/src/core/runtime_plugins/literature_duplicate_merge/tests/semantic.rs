//! Deployed-schema/SQL-log tests; the identity resources are instrumented only
//! in this test fixture where explicitly noted, never in product registration.
use super::{Fixture, MergeCause, MergeOutcome};
use crate::runtime_plugins::literature_duplicate_undo::tests::fixture::contract_reply;
use crate::runtime_plugins::{RuntimePluginCatalog, data::AcceptedDataResource};
use research_workbench_native_sdk::{
    data::{DataAccess, DataField, DataResourceGrant, DataType},
    knowledge_snapshot::KnowledgeConcept,
    nal_bootstrap::BootstrapPhaseV1,
};
use rusqlite::{Connection, functions::FunctionFlags, hooks::AuthAction, hooks::AuthContext};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn configured(f: &Fixture) {
    f.data.db.lock().unwrap().execute(
        "update ai_provider_settings set enabled=1,region='cn-beijing',model_id='text-embedding-v4' where provider_id='qwen-embedding'",
        [],
    ).unwrap();
}

fn semantic_state(db: &Connection) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    for (name, sql) in [
        (
            "representations",
            "select json_array(id,subject_id,subject_revision,language,render_version,rendered_text,input_sha256,created_at) from semantic_representations order by id",
        ),
        (
            "spaces",
            "select json_array(id,provider_id,region,model_id,model_revision,dimension,instruction_set_version,created_at) from embedding_spaces order by id",
        ),
        (
            "generations",
            "select json_array(id,namespace,embedding_space_id,generation,status,relative_database_path,created_at,activated_at) from embedding_index_generations order by id",
        ),
        (
            "records",
            "select json_array(id,representation_id,embedding_space_id,index_generation_id,lance_table,lance_row_id,status,created_at,updated_at) from embedding_records order by id",
        ),
        (
            "jobs",
            "select json_array(id,embedding_record_id,attempt,status,idempotency_key,created_at) from embedding_outbox_jobs order by id",
        ),
    ] {
        let rows = db
            .prepare(sql)
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        result.insert(name.into(), serde_json::json!(rows));
    }
    serde_json::Value::Object(result)
}

#[test]
fn configured_merge_creates_actual_space_generation_and_canonical_queue_identities() {
    for expected_generation in [1, i64::MAX] {
        let f = Fixture::new();
        f.collection();
        configured(&f);
        let space = embedding_space_id("cn-beijing", "text-embedding-v4");
        if expected_generation == i64::MAX {
            seed_active_generation(&f, &space, expected_generation);
        }
        let note = f.data.note();
        f.apply().unwrap();
        let before = {
            let db = f.data.db.lock().unwrap();
            let records = db.prepare(
            "select r.representation_id,r.embedding_space_id,g.generation,r.id,r.lance_row_id,j.idempotency_key
             from embedding_records r join embedding_index_generations g on g.id=r.index_generation_id
             join embedding_outbox_jobs j on j.embedding_record_id=r.id order by r.id",
        ).unwrap().query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?)))
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            // A collection imports exactly membership/tagged/its own concept.
            assert_eq!(records.len(), 3);
            for (representation, actual_space, generation, record, row_id, idempotency) in records {
                assert_eq!(actual_space, space);
                assert_eq!(generation, expected_generation);
                assert_eq!(row_id, stable_row_id(&representation, &space, generation));
                assert_eq!(idempotency, format!("embedding:{record}:1"));
            }
            let snapshots = db.prepare(
            "select snapshot_json from research_object_revisions where object_kind='concept' order by object_id",
        ).unwrap().query_map([], |r| r.get::<_,String>(0)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap();
            for snapshot in snapshots {
                let concept: KnowledgeConcept = serde_json::from_str(&snapshot).unwrap();
                let expected = render_concept(&concept).unwrap();
                let actual: (String,String) = db.query_row(
                "select rendered_text,input_sha256 from semantic_representations where subject_id=?1",
                [&concept.id], |r| Ok((r.get(0)?,r.get(1)?)),
            ).unwrap();
                assert_eq!(actual.0, expected);
                assert_eq!(
                    actual.1,
                    format!("{:x}", Sha256::digest(expected.as_bytes()))
                );
            }
            assert_eq!(
                db.query_row(
                    "select count(*) from embedding_index_generations",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                db.query_row(
                    "select relative_database_path from embedding_index_generations",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                if expected_generation == 1 {
                    format!("{space}/generation-000001")
                } else {
                    "synthetic-existing".into()
                }
            );
            semantic_state(&db)
        };
        assert_eq!(f.data.note(), note);
        let data = f.data.reopen();
        assert_eq!(semantic_state(&data.db.lock().unwrap()), before);
        assert_eq!(data.note(), note);
    }
}
fn seed_active_generation(f: &Fixture, space: &str, generation: i64) {
    let db = f.data.db.lock().unwrap();
    db.execute("insert into embedding_spaces(id,provider_id,region,model_id,model_revision,dimension,scalar_type,distance_metric,normalization,document_input_role,query_input_role,instruction_set_version,created_at)
            values(?1,'qwen-embedding','cn-beijing','text-embedding-v4','text-embedding-v4',1024,'float32','cosine','l2','document','query','concept-identity-v1','synthetic')",[&space]).unwrap();
    db.execute("insert into embedding_index_generations(id,namespace,embedding_space_id,backend_kind,generation,status,relative_database_path,lance_table,index_kind,config_json,created_at,activated_at)
            values('existing-generation','accepted_concepts',?1,'lancedb',?2,'active','synthetic-existing','accepted_concepts','exact','{}','synthetic','synthetic')",rusqlite::params![space,generation]).unwrap();
}

/// Only these synthetic accepted fixture resources call observing UDFs. The
/// product SQL is unchanged and has separate original-byte/expression proof.
fn observe_identities(f: &mut Fixture) -> (tempfile::TempDir, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("literature");
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(
        root.join("manifest.json"),
        r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
    )
    .unwrap();
    std::fs::write(root.join("index.js"), "export {};\n").unwrap();
    let row_sql = "select test_merge_row_id(?1,?2,?3)";
    let job_sql = "select test_merge_idempotency(?1)";
    std::fs::write(root.join("data/row.sql"), row_sql).unwrap();
    std::fs::write(root.join("data/job.sql"), job_sql).unwrap();
    let catalog = RuntimePluginCatalog::new(Some(directory.path().to_owned()));
    let package = catalog.discover().unwrap().packages.remove(0);
    let field = |data_type| DataField {
        data_type,
        nullable: false,
    };
    let load = |id: &str, path: &str, sql: &str, parameters| {
        AcceptedDataResource::load(
            &catalog,
            &package,
            "literature_library",
            path,
            DataResourceGrant {
                id: id.into(),
                version: 1,
                sha256: format!("{:x}", Sha256::digest(sql.as_bytes())),
                access: DataAccess::Read,
                parameters,
                columns: vec![field(DataType::Text)],
            },
        )
        .unwrap()
    };
    f.resources.rules.semantic_record_row_id = load(
        "literature.test.merge-row",
        "data/row.sql",
        row_sql,
        vec![
            field(DataType::Text),
            field(DataType::Text),
            field(DataType::Integer),
        ],
    );
    f.resources.rules.semantic_job_idempotency = load(
        "literature.test.merge-job",
        "data/job.sql",
        job_sql,
        vec![field(DataType::Text)],
    );
    let rows = Arc::new(AtomicUsize::new(0));
    let jobs = Arc::new(AtomicUsize::new(0));
    let row_counter = rows.clone();
    let job_counter = jobs.clone();
    let db = f.data.db.lock().unwrap();
    db.create_scalar_function(
        "test_merge_row_id",
        3,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            row_counter.fetch_add(1, Ordering::SeqCst);
            Ok(stable_row_id(
                &ctx.get::<String>(0)?,
                &ctx.get::<String>(1)?,
                ctx.get(2)?,
            ))
        },
    )
    .unwrap();
    db.create_scalar_function(
        "test_merge_idempotency",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            job_counter.fetch_add(1, Ordering::SeqCst);
            Ok(format!("embedding:{}:1", ctx.get::<String>(0)?))
        },
    )
    .unwrap();
    (directory, rows, jobs)
}

#[test]
fn existing_representation_still_reads_settings_and_queue_but_skips_new_identities() {
    let mut f = Fixture::new();
    f.collection();
    configured(&f);
    let space = embedding_space_id("cn-beijing", "text-embedding-v4");
    seed_active_generation(&f, &space, i64::MAX);
    {
        let db = f.data.db.lock().unwrap();
        // Negative values are not a deployed-schema input; retain its CHECK.
        let rejected=db.execute("insert into embedding_index_generations(id,namespace,embedding_space_id,backend_kind,generation,status,relative_database_path,lance_table,index_kind,config_json,created_at,activated_at)
            values('negative','accepted_concepts',?1,'lancedb',-1,'building','synthetic','accepted_concepts','exact','{}','synthetic','synthetic')",[&space]).unwrap_err();
        assert!(
            matches!(rejected,rusqlite::Error::SqliteFailure(code,_) if code.extended_code==rusqlite::ffi::SQLITE_CONSTRAINT_CHECK)
        );
        db.execute_batch(&format!("create trigger seed_existing_semantic after insert on concepts begin
            insert into semantic_representations(id,subject_kind,subject_id,subject_revision,representation_kind,namespace,language,render_version,rendered_text,input_sha256,created_at)
            values('old-repr-'||new.id,'concept',new.id,new.revision,'concept_identity','accepted_concepts','','concept-identity-v1','Synthetic existing representation','{}','synthetic');
            insert into embedding_records(id,representation_id,embedding_space_id,index_generation_id,lance_table,lance_row_id,vector_encoding,status,created_at,updated_at)
            values('old-record-'||new.id,'old-repr-'||new.id,'{space}','existing-generation','accepted_concepts','old-row-'||new.id,'float32','queued','synthetic','synthetic');
            insert into embedding_outbox_jobs(id,embedding_record_id,attempt,status,idempotency_key,created_at)
            values('old-job-'||new.id,'old-record-'||new.id,3,'failed','existing:'||new.id,'synthetic'); end;", "a".repeat(64))).unwrap();
    }
    let (_instrumented_package, row_ids, job_ids) = observe_identities(&mut f);
    let settings = Arc::new(AtomicUsize::new(0));
    let queue = Arc::new(AtomicUsize::new(0));
    let observed_settings = settings.clone();
    let observed_queue = queue.clone();
    f.data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |ctx: AuthContext<'_>| {
            if let AuthAction::Read { table_name, .. } = ctx.action {
                if table_name == "ai_provider_settings" {
                    observed_settings.fetch_add(1, Ordering::SeqCst);
                }
                if table_name == "embedding_outbox_jobs" {
                    observed_queue.fetch_add(1, Ordering::SeqCst);
                }
            }
            rusqlite::hooks::Authorization::Allow
        }));
    f.apply().unwrap();
    f.data
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> rusqlite::hooks::Authorization>);
    assert!(settings.load(Ordering::SeqCst) > 0);
    assert!(queue.load(Ordering::SeqCst) > 0);
    assert_eq!(row_ids.load(Ordering::SeqCst), 0);
    assert_eq!(job_ids.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.scalar::<i64>("select count(*) from semantic_representations where id like 'old-repr-%'"),
        3
    );
    assert_eq!(f.scalar::<i64>("select count(*) from embedding_records where id like 'old-record-%' and lance_row_id like 'old-row-%'"), 3);
    assert_eq!(f.scalar::<i64>("select count(*) from embedding_outbox_jobs where id like 'old-job-%' and attempt=3 and status='failed'"), 3);
    assert_eq!(
        f.scalar::<i64>("select generation from embedding_index_generations"),
        i64::MAX
    );
}

#[test]
fn record_insert_failure_precedes_the_instrumented_idempotency_expression_and_rolls_back() {
    let mut f = Fixture::new();
    f.collection();
    configured(&f);
    let note = f.data.note();
    let (_instrumented_package, rows, jobs) = observe_identities(&mut f);
    f.data
        .db
        .lock()
        .unwrap()
        .execute_batch(
            "create trigger reject_semantic_record before insert on embedding_records begin
        select raise(abort,'synthetic-record-rejected'); end;",
        )
        .unwrap();
    let error = f.apply().unwrap_err();
    assert_eq!(error.outcome, MergeOutcome::RolledBack);
    assert_eq!(error.problem.code(), "storage_error");
    let MergeCause::Nal(problem) = error.problem.cause else {
        panic!("expected original NAL failure");
    };
    assert_eq!(problem.phase, BootstrapPhaseV1::RecordInsert);
    assert_eq!(rows.load(Ordering::SeqCst), 1);
    assert_eq!(jobs.load(Ordering::SeqCst), 0);
    for table in [
        "terms",
        "concepts",
        "semantic_representations",
        "embedding_spaces",
        "embedding_index_generations",
        "embedding_records",
        "embedding_outbox_jobs",
        "merge_operations",
    ] {
        assert_eq!(
            f.scalar::<i64>(&format!("select count(*) from {table}")),
            0,
            "{table}"
        );
    }
    assert_eq!(
        f.scalar::<i64>(
            "select count(*) from bibliographic_items where id='source' and archived_at is null"
        ),
        1
    );
    assert_eq!(f.data.note(), note);
    let data = f.data.reopen();
    assert_eq!(data.note(), note);
    assert_eq!(
        data.db
            .lock()
            .unwrap()
            .query_row("select count(*) from embedding_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn disabled_and_empty_model_keep_representations_without_creating_embedding_state() {
    for (enabled, model) in [(false, "text-embedding-v4"), (true, "   ")] {
        let f = Fixture::new();
        f.collection();
        f.data.db.lock().unwrap().execute(
            "update ai_provider_settings set enabled=?1,region='cn-beijing',model_id=?2 where provider_id='qwen-embedding'",
            rusqlite::params![enabled,model],
        ).unwrap();
        f.apply().unwrap();
        assert_eq!(
            f.scalar::<i64>("select count(*) from semantic_representations"),
            3
        );
        for table in [
            "embedding_spaces",
            "embedding_index_generations",
            "embedding_records",
            "embedding_outbox_jobs",
        ] {
            assert_eq!(
                f.scalar::<i64>(&format!("select count(*) from {table}")),
                0,
                "{enabled}/{table}"
            );
        }
    }
}

fn embedding_space_id(region: &str, model: &str) -> String {
    serde_json::from_value(contract_reply("space", serde_json::json!([region, model]))).unwrap()
}
fn stable_row_id(representation: &str, space: &str, generation: i64) -> String {
    let db = Connection::open_in_memory().unwrap();
    crate::data_interface::value_functions::register(&db).unwrap();
    let sql = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-resources-v1/literature/duplicate-merge-semantic-record-row-id.v1.sql"
    ));
    // Check this fixed resource against the independent public Native samples.
    for case in crate::runtime_plugins::literature_duplicate_undo::tests::fixture::recorded_cases()
        .into_iter()
        .filter(|case| case["kind"] == "row")
    {
        let input = &case["input"];
        let actual: String = db
            .query_row(
                sql,
                rusqlite::params![
                    input[0].as_str().unwrap(),
                    input[1].as_str().unwrap(),
                    input[2].as_i64().unwrap()
                ],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(actual, case["output"].as_str().unwrap());
    }
    db.query_row(
        sql,
        rusqlite::params![representation, space, generation],
        |row| row.get(0),
    )
    .unwrap()
}
fn render_concept(concept: &KnowledgeConcept) -> Result<String, serde_json::Error> {
    serde_json::from_value(contract_reply("render", serde_json::to_value(concept)?))
}
