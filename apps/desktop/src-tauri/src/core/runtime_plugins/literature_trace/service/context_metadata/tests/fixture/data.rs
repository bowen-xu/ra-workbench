use super::*;
use crate::{
    agent::repository::submissions,
    runtime_events::RuntimeEvents,
    storage::{AppStorePaths, AppStorePorts, AppStoreSources},
};
use research_agent_core::{protocol::ModelIdentity, submission::SubmissionInput};
use std::path::Path;

pub(super) fn open(path: &Path) -> AppStore {
    AppStore::open_with_paths(
        AppStorePaths {
            data_directory: path.into(),
            agent_configuration_directory: path.join("configuration"),
        },
        AppStorePorts::with_events(
            RuntimeEvents::new(|_, _| Ok(())),
            AppStoreSources::default(),
        ),
    )
    .unwrap()
}
pub(super) fn seed(store: &AppStore, paper: &str) -> (String, String) {
    let db = store.connection().unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle','Metadata Ω','now','now')",[paper]).unwrap();
    db.execute("insert into creators(id,family_name,given_name,literal_name) values('creator','Family','Given','')",[]).unwrap();
    db.execute("insert into item_creators(item_id,creator_id,role,position) values(?1,'creator','author',0)",[paper]).unwrap();
    db.execute("insert into identifiers(id,item_id,kind,display_value,normalized_value) values('identifier',?1,'url','https://example.invalid/Ω','https://example.invalid/Ω')",[paper]).unwrap();
    let note =
        crate::notes::insert_reading_fixture(&db, &uuid::Uuid::new_v4().to_string(), "Note Ω");
    db.execute(
        "update reading_note_sources set paper_id_snapshot=?2 where page_id=?1",
        rusqlite::params![note, paper],
    )
    .unwrap();
    let portrait = crate::structured_resources::ensure_paper_resources(&db, paper)
        .unwrap()
        .into_iter()
        .find(|resource| resource.kind == "portrait")
        .unwrap()
        .id;
    // Ordinary source reading accepts the original saved JSON, without a typed
    // portrait-validation request or a substitute Native notes representation.
    db.execute(
        "update structured_resources set content_json=?1,baseline_json=?1 where id=?2",
        rusqlite::params![json!({"description":"Portrait Ω"}).to_string(), portrait],
    )
    .unwrap();
    (note, portrait)
}
pub(super) fn submission(store: &AppStore, resources: Vec<Resource>) -> SubmissionRecord {
    let mut db = store.connection().unwrap();
    let id = uuid::Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Metadata tool fixture')",
        [id.to_string()],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    let context = resource_grants::grants(&tx, &resources, None).unwrap();
    let input = SubmissionInput {
        remote_compaction_v2: None,
        text: "Read selected literature sources".into(),
        user_input: vec![],
        skill_instructions: vec![],
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        context,
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
    };
    let record = submissions::insert_in(&tx, id, uuid::Uuid::new_v4(), input).unwrap();
    tx.commit().unwrap();
    record
}

pub(super) fn budget_notes(store: &AppStore, first: &str) -> String {
    let db = store.connection().unwrap();
    let second = crate::notes::insert_reading_fixture(
        &db,
        &uuid::Uuid::new_v4().to_string(),
        "Budget fixture",
    );
    db.execute(
        "update reading_note_sources set paper_id_snapshot='paper' where page_id=?1",
        [&second],
    )
    .unwrap();
    // Each Note stays below its own 8 MiB snapshot/view limit and each text item
    // below 2,000,000 bytes. The two actual source views together exceed the
    // existing search budget only after both confirmed Note reads.
    let text = "x".repeat(1_500_000);
    let content =
        json!({"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]}).to_string();
    for page in [first, second.as_str()] {
        db.execute(
            "update note_blocks set content_json=?1,plain_text=?2 where page_id=?3",
            rusqlite::params![content, text, page],
        )
        .unwrap();
        for position in [1, 2] {
            db.execute("insert into note_blocks(id,scope_kind,page_id,position,block_type,content_json,plain_text,created_at,updated_at) values(?1,'page',?2,?3,'paragraph',?4,?5,'now','now')", rusqlite::params![uuid::Uuid::new_v4().to_string(),page,position,content,text]).unwrap();
        }
    }
    second
}
