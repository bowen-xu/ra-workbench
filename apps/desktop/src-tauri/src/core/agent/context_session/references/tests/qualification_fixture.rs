use crate::{
    agent::context_session::{
        PrepareRequest,
        sources::{Access, Part, Resource},
    },
    runtime_events::RuntimeEvents,
    storage::{AppStore, AppStorePaths, AppStorePorts, AppStoreSources},
};
use research_agent_core::{
    protocol::ModelIdentity,
    user_input::{MentionTarget, TextElement, UserInput},
};
use serde_json::json;
use std::path::Path;
use uuid::Uuid;

pub(super) const NOTE_BODY: &str = "Saved Core note 原文 Ω";
pub(super) const PORTRAIT_BODY: &str = "Saved Core portrait body Ω";

pub(super) struct Fixture {
    pub root: tempfile::TempDir,
    pub store: AppStore,
    pub conversation: Uuid,
    pub note: Uuid,
    pub foreign_note: Uuid,
    pub portrait: String,
}

pub(super) fn open(root: &Path) -> AppStore {
    let store = AppStore::open_with_paths(
        AppStorePaths {
            data_directory: root.join("store"),
            agent_configuration_directory: root.join("configuration"),
        },
        AppStorePorts::with_events(
            RuntimeEvents::new(|_, _| Ok(())),
            AppStoreSources::default(),
        ),
    )
    .unwrap();
    assert!(store.paper_metadata_source().is_none());
    assert!(store.paper_portrait_source().is_none());
    assert!(store.paper_reading_source().is_none());
    assert!(store.retrieval_trace_source().is_none());
    assert!(store.writing_resource_source().is_none());
    assert!(store.native_note_trace().is_none());
    store
}

impl Fixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = open(root.path());
        let conversation = Uuid::new_v4();
        let note = Uuid::new_v4();
        let foreign_note = Uuid::new_v4();
        let portrait = {
            let mut db = store.connection().unwrap();
            db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','document','Selected synthetic paper','now','now'),('other','document','Other synthetic paper','now','now')", []).unwrap();
            db.execute(
                "insert into agent_conversations(id,title) values(?1,'Context qualification')",
                [conversation.to_string()],
            )
            .unwrap();
            let tx = db.transaction().unwrap();
            for (id, paper) in [(note, "paper"), (foreign_note, "other")] {
                crate::notes::create_page_with_paper_in(
                    &tx,
                    crate::notes::NotePageDraft {
                        title: format!("Synthetic Note {paper}"),
                        parent_page_id: None,
                        parent_block_position: None,
                    },
                    Some(paper),
                    Some(&id.to_string()),
                )
                .unwrap();
                save_note(&tx, id, 0, NOTE_BODY);
            }
            tx.commit().unwrap();
            let portrait = crate::structured_resources::ensure_paper_resources(&db, "paper")
                .unwrap()
                .into_iter()
                .find(|resource| resource.kind == "portrait")
                .unwrap()
                .id;
            // Synthetic stored content distinguishes an identity-only grant from
            // eager body materialization. This fixture does not validate portraits.
            db.execute(
                "update structured_resources set content_json=?1,baseline_json=?1 where id=?2",
                rusqlite::params![json!({"description": PORTRAIT_BODY}).to_string(), portrait],
            )
            .unwrap();
            portrait
        };
        Self {
            root,
            store,
            conversation,
            note,
            foreign_note,
            portrait,
        }
    }

    pub fn group(&self, dynamic: bool, excluded: bool) -> Resource {
        Resource::PaperGroup {
            resource_id: "paper".into(),
            new_member_access: dynamic.then_some(Access::ReadWrite),
            group_access: Some(Access::ReadOnly),
            parts: vec![
                Part {
                    resource: Resource::PaperNotes {
                        resource_id: "paper".into(),
                        excluded_note_ids: excluded
                            .then(|| self.note.to_string())
                            .into_iter()
                            .collect(),
                    },
                    label: "阅读笔记".into(),
                    selected: true,
                    access: Access::ReadWrite,
                },
                Part {
                    resource: Resource::StructuredObject {
                        resource_id: self.portrait.clone(),
                        access: None,
                    },
                    label: "画像".into(),
                    selected: !excluded,
                    access: Access::ReadWrite,
                },
            ],
        }
    }

    pub fn request(&self, resources: Vec<Resource>, user_input: Vec<UserInput>) -> PrepareRequest {
        let mut text = "Inspect the selected synthetic sources".to_owned();
        let mut text_elements = Vec::new();
        for (index, item) in user_input.iter().enumerate() {
            let UserInput::Mention { name, .. } = item else {
                panic!("qualification fixture expects file mentions");
            };
            text.push(' ');
            let start_utf16 = text.encode_utf16().count();
            text.push('@');
            text.push_str(name);
            text_elements.push(TextElement {
                start_utf16,
                end_utf16: text.encode_utf16().count(),
                item_index: index + 1,
            });
        }
        let mut input = vec![UserInput::Text {
            text: text.clone(),
            text_elements,
        }];
        input.extend(user_input);
        PrepareRequest {
            conversation_id: self.conversation,
            request_id: Uuid::new_v4(),
            text,
            user_input: input,
            resources,
            retrieval: None,
            model: None,
            queue: None,
        }
    }
}

pub(super) fn save_note(db: &rusqlite::Transaction<'_>, id: Uuid, revision: i64, text: &str) {
    let mut document = crate::notes::load_document_value(db, &id.to_string()).unwrap();
    document.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]});
    crate::notes::save_document_in(
        db,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: revision,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
}

pub(super) fn mention(resource: &Resource) -> UserInput {
    let encoded = serde_json::to_value(resource).unwrap();
    UserInput::Mention {
        name: "Synthetic file mention Ω".into(),
        target: MentionTarget::File {
            source_id: format!("context:{encoded}"),
            resource_id: encoded["resourceId"].as_str().unwrap().into(),
            source_url: None,
            revision: String::new(),
        },
    }
}

pub(super) fn model() -> (ModelIdentity, Option<String>, Option<String>) {
    (
        ModelIdentity {
            provider_id: "qualification-fixture".into(),
            model_id: "qualification-fixture".into(),
            revision: None,
        },
        None,
        None,
    )
}
