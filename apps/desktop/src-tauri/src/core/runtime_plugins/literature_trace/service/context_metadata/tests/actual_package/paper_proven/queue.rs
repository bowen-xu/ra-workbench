//! Original Core paper queue APIs, using the same accepted paper and conversation.
use super::*;
use crate::agent::paper_queue;

pub(super) struct QueueProof {
    input: Uuid,
    add: Value,
    added: Value,
}
async fn current(f: &Fixture, paper: &str) -> Value {
    serde_json::to_value(
        paper_queue::read(
            &f.store,
            paper_queue::ReadRequest {
                conversation_id: f.record.conversation_id,
                paper_id: paper.into(),
            },
        )
        .await
        .unwrap(),
    )
    .unwrap()
}
async fn edit(f: &Fixture, request: &Value) -> Value {
    serde_json::to_value(
        paper_queue::edit(&f.store, serde_json::from_value(request.clone()).unwrap())
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn status(f: &Fixture, request: &Value) -> Value {
    serde_json::to_value(
        paper_queue::edit_status(&f.store, serde_json::from_value(request.clone()).unwrap())
            .await
            .unwrap(),
    )
    .unwrap()
}
impl QueueProof {
    pub(super) async fn enqueue(f: &Fixture, paper: &str) -> Self {
        let before = current(f, paper).await;
        let input = Uuid::new_v4();
        let add = json!({"conversationId":f.record.conversation_id,"paperId":paper,
            "requestId":Uuid::new_v4(),"expectedRevision":before["revision"],
            "edit":{"type":"add","inputId":input,"text":"Read this paper Ω"}});
        let added = edit(f, &add).await;
        assert_eq!(added["inputs"].as_array().unwrap().len(), 1);
        assert_eq!(added["inputs"][0]["inputId"], input.to_string());
        assert_eq!(
            added["inputs"][0]["origin"],
            json!({"kind":"paper","resourceId":paper})
        );
        let receipt = status(f, &add).await;
        assert_eq!(receipt["status"]["type"], "committed");
        assert_eq!(receipt["status"]["queue"]["inputs"], added["inputs"]);
        // A queue stores user input. Current source content is frozen only when
        // its original production submission path prepares the selected paper.
        f.store.connection().unwrap().execute("update docling_parse_runs set json_content=json_set(json_content,'$.texts[0].text','Queued current evidence Ω') where id='prepare-parse'", []).unwrap();
        Self { input, add, added }
    }
    pub(super) async fn after_reopen(&self, f: &Fixture, paper: &str) {
        assert_eq!(current(f, paper).await, self.added);
        let receipt = status(f, &self.add).await;
        assert_eq!(receipt["status"]["type"], "committed");
        assert_eq!(receipt["intent"]["requestId"], self.add["requestId"]);
        assert_eq!(edit(f, &self.add).await, self.added);
        assert_eq!(
            current(f, paper).await["inputs"].as_array().unwrap().len(),
            1
        );
    }
    pub(super) async fn after_start(&self, f: &Fixture, paper: &str) {
        let before = current(f, paper).await;
        assert_eq!(before["inputs"][0]["inputId"], self.input.to_string());
        let remove = json!({"conversationId":f.record.conversation_id,"paperId":paper,
            "requestId":Uuid::new_v4(),"expectedRevision":before["revision"],
            "edit":{"type":"remove","inputId":self.input}});
        let removed = edit(f, &remove).await;
        assert!(removed["inputs"].as_array().unwrap().is_empty());
        assert_eq!(edit(f, &remove).await, removed);
        let receipt = status(f, &remove).await;
        assert_eq!(receipt["status"]["type"], "committed");
        assert_eq!(receipt["status"]["queue"]["inputs"], json!([]));
        // Replaying the original add returns its historical receipt, while
        // current Q retains the proven removal and never adds a second input.
        assert_eq!(edit(f, &self.add).await, self.added);
        assert!(
            current(f, paper).await["inputs"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
