//! Real production entry points, real managed pair, real serde wire samples.
use crate::{
    agent::{
        context_session::queue as context, note_queue, paper_queue, writing_queue,
        writing_submission::WritingSubmissionService,
    },
    data_interface::Database,
    storage::AppStore,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
enum Host {
    Writing,
    Paper,
    Note,
    ReadingNote,
    Context,
}
struct Fixture {
    root: tempfile::TempDir,
    store: AppStore,
    manuscript: Uuid,
    paper: Uuid,
    note: Uuid,
    reading: Uuid,
    conversations: [Uuid; 5],
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("queue.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let manuscript = Uuid::new_v4();
        // Library IDs remain String DTOs, while the existing queue provenance
        // contract requires their stored resource ID to be a non-nil UUID.
        let paper = Uuid::new_v4();
        let note = Uuid::new_v4();
        let reading = Uuid::new_v4();
        sql.execute("insert into research_projects(id,title,summary,created_at) values('p','Synthetic','','now')",[]).unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'p','Synthetic','main.tex','now','now')",[manuscript.to_string()]).unwrap();
        sql.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle','Synthetic','now','now')",[paper.to_string()]).unwrap();
        sql.execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'Synthetic','now','now')",[note.to_string()]).unwrap();
        crate::notes::insert_reading_fixture(&sql, &reading.to_string(), "Synthetic reading note");
        let conversations = std::array::from_fn(|_| Uuid::new_v4());
        for (i, c) in conversations.iter().enumerate() {
            sql.execute(
                "insert into agent_conversations(id,title) values(?1,'Synthetic queue status')",
                [c.to_string()],
            )
            .unwrap();
            let binding = match i {
                0 => Some(("manuscript", manuscript.to_string())),
                1 => Some(("paper", paper.to_string())),
                2 => Some(("note", note.to_string())),
                3 => Some(("readingNote", reading.to_string())),
                _ => None,
            };
            if let Some((kind, id)) = binding {
                sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,?2,?3)",params![c.to_string(),kind,id]).unwrap();
            }
        }
        let db = Database::open(sql, &root.path().join("log")).unwrap();
        let store = AppStore::for_test_database(db, root.path().join("data/objects"));
        Self {
            root,
            store,
            manuscript,
            paper,
            note,
            reading,
            conversations,
        }
    }
    fn request(&self, host: Host, c: Uuid, r: Uuid, revision: u64, edit: Value) -> Value {
        let mut v =
            json!({"conversationId":c,"requestId":r,"expectedRevision":revision,"edit":edit});
        match host {
            Host::Writing => v["manuscriptId"] = json!(self.manuscript),
            Host::Paper => v["paperId"] = json!(self.paper),
            Host::Note => v["scope"] = json!({"kind":"note","resourceId":self.note}),
            Host::ReadingNote => {
                v["scope"] = json!({"kind":"readingNote","resourceId":self.reading})
            }
            Host::Context => {}
        }
        v
    }
    fn reopen(self) -> Self {
        let Self {
            root,
            store,
            manuscript,
            paper,
            note,
            reading,
            conversations,
        } = self;
        drop(store);
        let db = Database::open(
            Connection::open(root.path().join("queue.sqlite3")).unwrap(),
            &root.path().join("log"),
        )
        .unwrap();
        let store = AppStore::for_test_database(db, root.path().join("data/objects"));
        Self {
            root,
            store,
            manuscript,
            paper,
            note,
            reading,
            conversations,
        }
    }
}
async fn edit(f: &Fixture, host: Host, v: Value) -> Result<Value, super::super::CommandError> {
    Ok(match host {
        Host::Writing => serde_json::to_value(
            writing_queue::edit(
                &WritingSubmissionService::from_app_store(&f.store),
                serde_json::from_value(v).unwrap(),
            )
            .await?,
        )
        .unwrap(),
        Host::Paper => serde_json::to_value(
            paper_queue::edit(&f.store, serde_json::from_value(v).unwrap()).await?,
        )
        .unwrap(),
        Host::Note | Host::ReadingNote => serde_json::to_value(
            note_queue::edit(&f.store, serde_json::from_value(v).unwrap()).await?,
        )
        .unwrap(),
        Host::Context => serde_json::to_value(
            context::edit_queue(&f.store, serde_json::from_value(v).unwrap()).await?,
        )
        .unwrap(),
    })
}
async fn status(f: &Fixture, host: Host, v: Value) -> Result<Value, super::super::CommandError> {
    let response = match host {
        Host::Writing => {
            writing_queue::edit_status(
                &WritingSubmissionService::from_app_store(&f.store),
                serde_json::from_value(v).unwrap(),
            )
            .await?
        }
        Host::Paper => {
            paper_queue::edit_status(&f.store, serde_json::from_value(v).unwrap()).await?
        }
        Host::Note | Host::ReadingNote => {
            note_queue::edit_status(&f.store, serde_json::from_value(v).unwrap()).await?
        }
        Host::Context => context::edit_status(&f.store, serde_json::from_value(v).unwrap()).await?,
    };
    Ok(serde_json::to_value(response).unwrap())
}

fn confirmed<T>(result: Result<T, super::super::CommandError>, host: Host, stage: &str) -> T {
    // A failure should identify its host and boundary without dumping an intent
    // or document/tool payload into the test log.
    result.unwrap_or_else(|_| panic!("native queue {host:?} failed at {stage}"))
}

#[tokio::test]
async fn five_scope_native_entry_points_and_cold_status_keep_original_intent_and_frozen_queue() {
    let f = Fixture::new();
    let hosts = [
        Host::Writing,
        Host::Paper,
        Host::Note,
        Host::ReadingNote,
        Host::Context,
    ];
    let mut requests = vec![];
    let mut golden = vec![];
    for (host, c) in hosts.into_iter().zip(f.conversations) {
        let request=f.request(host,c,Uuid::new_v4(),0,json!({"type":"add","inputId":Uuid::new_v4(),"text":"Synthetic retained input","userInput":[]}));
        let before = confirmed(
            status(&f, host, request.clone()).await,
            host,
            "initial status",
        );
        assert_eq!(before["status"]["type"], "notFound");
        let written = confirmed(edit(&f, host, request.clone()).await, host, "fresh edit");
        let known = confirmed(
            status(&f, host, request.clone()).await,
            host,
            "committed status",
        );
        assert_eq!(known["status"]["type"], "committed");
        assert_eq!(known["status"]["queue"]["inputs"], written["inputs"]);
        assert_eq!(known["status"]["proof"]["sqlFence"], "0");
        assert!(known["status"]["proof"]["result"].is_null());
        assert_eq!(known["intent"]["edit"], request["edit"]);
        let pause = f.request(host, c, Uuid::new_v4(), 1, json!({"type":"pause"}));
        confirmed(edit(&f, host, pause).await, host, "later pause");
        assert_eq!(
            confirmed(
                status(&f, host, request.clone()).await,
                host,
                "historical status"
            )["status"]["queue"]["paused"],
            false
        );
        let mut wrong = request.clone();
        wrong["expectedRevision"] = json!(1);
        let failure = match status(&f, host, wrong).await {
            Err(error) => serde_json::to_value(error).unwrap(),
            Ok(_) => panic!("native queue {host:?} accepted a changed original revision"),
        };
        assert_eq!(failure["code"], "QUEUE_STORAGE_FAILURE");
        assert_eq!(failure["reason"]["code"], "DATA_SQL_OPERATION_CONFLICT");
        golden.push(
            json!({"request":request,"notFound":before,"committed":known,"storageFailure":failure}),
        );
        requests.push((host, request));
    }
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    let f = f.reopen();
    for (host, request) in requests {
        let known = confirmed(status(&f, host, request).await, host, "cold status");
        assert_eq!(known["status"]["type"], "committed");
        assert_eq!(known["status"]["queue"]["revision"], 1);
    }
    assert_eq!(f.store.connection().unwrap().query_row("select count(*) from data_sql_operation_receipts where kind='agent.input_queue.edit'",[],|row|row.get::<_,u64>(0)).unwrap(),10);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_submissions", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    println!(
        "QUEUE_EDIT_NATIVE_GOLDEN={}",
        json!({"schemaVersion":1,"scopes":golden})
    );
}
