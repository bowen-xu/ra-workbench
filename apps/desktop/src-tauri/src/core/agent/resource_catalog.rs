//! Host-bound resource discovery. Returns identities, never renderer file grants.
use crate::storage::AppStore;
use research_agent_core::{
    submission::ContextText,
    user_input::{MentionTarget, UserInput},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub conversation_id: Uuid,
    pub kind: String,
    pub resource_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub group: &'static str,
    pub label: String,
    pub source: String,
    pub input: UserInput,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Error {
    Invalid,
    Unbound,
    Unavailable,
    #[serde(untagged)]
    Native(Box<crate::runtime_plugins::native::NativeHostError>),
}

#[tauri::command]
pub(crate) async fn agent_list_host_resources(
    store: State<'_, AppStore>,
    request: Request,
) -> Result<Vec<Resource>, Error> {
    list_current(store.inner().clone(), request).await
}
pub(crate) enum CatalogRead<T> {
    Ready(T),
    Need(Box<research_workbench_native_sdk::writing_project::Input>),
}

pub(crate) async fn list_current(
    store: AppStore,
    request: Request,
) -> Result<Vec<Resource>, Error> {
    if request.kind == "manuscript"
        && let Some(binding) = store.native_writing_context().cloned()
    {
        return crate::runtime_plugins::writing_projects::host_resources::list(
            &binding, store, request,
        )
        .await;
    }
    tokio::task::spawn_blocking(move || list(&store, &request))
        .await
        .map_err(|_| Error::Unavailable)?
}

fn validate_request(request: &Request) -> Result<(), Error> {
    if request.conversation_id.is_nil()
        || request.resource_id.trim().is_empty()
        || request.resource_id.len() > 1024
        || !matches!(
            request.kind.as_str(),
            "manuscript" | "paper" | "note" | "readingNote"
        )
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn validate_scope(db: &Connection, request: &Request) -> Result<(), Error> {
    let bound: bool = db.query_row("select exists(select 1 from agent_conversation_bindings b join agent_conversations c on c.id=b.conversation_id where b.conversation_id=?1 and b.resource_kind=?2 and b.resource_id=?3 and c.archived=0)", params![request.conversation_id.to_string(), request.kind, request.resource_id], |row| row.get(0)).map_err(|_| Error::Unavailable)?;
    if !bound {
        // A new draft has no persisted conversation yet, but its selected host
        // is already an application resource. Existing conversations may never
        // discover resources outside their bound host.
        let exists: bool = db
            .query_row(
                "select exists(select 1 from agent_conversations where id=?1)",
                [request.conversation_id.to_string()],
                |r| r.get(0),
            )
            .map_err(|_| Error::Unavailable)?;
        if exists {
            return Err(Error::Unbound);
        }
        use research_agent_core::conversation::BindingKind;
        let kind = match request.kind.as_str() {
            "manuscript" => BindingKind::Manuscript,
            "paper" => BindingKind::Paper,
            "note" => BindingKind::Note,
            "readingNote" => BindingKind::ReadingNote,
            _ => return Err(Error::Invalid),
        };
        let exists: bool = db
            .query_row(
                super::repository::binding_target_query(kind),
                [&request.resource_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::Unavailable)?;
        if !exists {
            return Err(Error::Unbound);
        }
    }
    Ok(())
}

pub(crate) fn authorize(store: &AppStore, request: &Request) -> Result<(), Error> {
    validate_request(request)?;
    let db = store.connection().map_err(|_| Error::Unavailable)?;
    validate_scope(&db, request)?;
    if request.kind == "manuscript" {
        Uuid::parse_str(&request.resource_id).map_err(|_| Error::Invalid)?;
    }
    Ok(())
}

pub(crate) fn list(store: &AppStore, request: &Request) -> Result<Vec<Resource>, Error> {
    let read = list_with_files(store, request, &mut |db, id| {
        let source = store.writing_resource_source().ok_or(Error::Unavailable)?;
        (source.list_host_files)(db, id)
            .map(CatalogRead::Ready)
            .map_err(|_| Error::Unavailable)
    })?;
    match read {
        CatalogRead::Ready(resources) => Ok(resources),
        CatalogRead::Need(_) => Err(Error::Unavailable),
    }
}

pub(crate) fn list_with_files(
    store: &AppStore,
    request: &Request,
    files: &mut impl FnMut(
        &Connection,
        &str,
    ) -> Result<
        CatalogRead<Vec<super::writing_resources::WritingResourceFile>>,
        Error,
    >,
) -> Result<CatalogRead<Vec<Resource>>, Error> {
    validate_request(request)?;
    let db = store.connection().map_err(|_| Error::Unavailable)?;
    validate_scope(&db, request)?;
    let mut resources = Vec::new();
    if request.kind == "manuscript" {
        let id = Uuid::parse_str(&request.resource_id).map_err(|_| Error::Invalid)?;
        let files = match files(&db, &request.resource_id)? {
            CatalogRead::Ready(files) => files,
            CatalogRead::Need(input) => return Ok(CatalogRead::Need(input)),
        };
        if files.len() > 256 {
            return Err(Error::Invalid);
        }
        resources.extend(
            files
                .into_iter()
                .filter(|file| file.kind != "image")
                .map(|file| Resource {
                    group: "files",
                    label: file.relative_path.clone(),
                    source: "当前稿件".into(),
                    input: UserInput::Mention {
                        name: file.relative_path.clone(),
                        target: MentionTarget::File {
                            source_id: format!(
                                "writing:{:x}",
                                Sha256::digest(format!("{id}/{}", file.relative_path).as_bytes())
                            ),
                            source_url: Some(research_agent_core::text_patch::address::object(
                                research_agent_core::text_patch::address::SourceRoot::Writings,
                                &std::iter::once(id.to_string())
                                    .chain(file.relative_path.split('/').map(str::to_owned))
                                    .collect::<Vec<_>>(),
                            )),
                            resource_id: file.relative_path,
                            revision: file.sha256,
                        },
                    },
                }),
        );
        let root = store
            .manuscript_directory()
            .join(id.to_string())
            .join("source");
        if root.is_dir() {
            let root = root.canonicalize().map_err(|_| Error::Unavailable)?;
            let catalog = super::skills::discovery::discover_under(
                &root,
                std::path::Path::new(".agents/skills"),
                &format!("manuscript:{id}"),
                8,
                |opened| {
                    super::repository::instruction_trust::is_trusted(&db, id, opened)
                        .map_err(|_| super::instructions::InstructionReadError::Unavailable)?
                        .then_some(())
                        .ok_or(super::instructions::InstructionReadError::Unauthorized)
                },
            );
            // Untrusted workspaces are not skill search roots.
            if let Ok(catalog) = catalog {
                for skill in catalog.skills {
                    resources.push(Resource {
                        group: "skills",
                        label: skill.name.clone(),
                        source: "当前稿件".into(),
                        input: UserInput::Skill {
                            name: skill.name,
                            identity: skill.identity,
                        },
                    });
                }
            }
        }
    } else {
        for context in contexts(
            store
                .require_paper_reading_source()
                .map_err(|_| Error::Unavailable)?,
            &db,
            request,
        )? {
            let path = if request.kind == "paper" {
                let anchor: serde_json::Value = context
                    .anchor
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok())
                    .unwrap_or_default();
                let mut parts = vec![request.resource_id.clone()];
                if let Some(file) = anchor["attachmentId"].as_str() {
                    parts.push(file.to_owned());
                }
                research_agent_core::text_patch::address::object(
                    research_agent_core::text_patch::address::SourceRoot::Library,
                    &parts,
                )
            } else {
                research_agent_core::text_patch::address::object(
                    research_agent_core::text_patch::address::SourceRoot::Notes,
                    std::slice::from_ref(&request.resource_id),
                )
            };
            resources.push(Resource {
                group: "files",
                label: context.resource_id.clone(),
                source: "当前宿主".into(),
                input: UserInput::Mention {
                    name: context.resource_id.clone(),
                    target: MentionTarget::File {
                        source_id: context.source_id,
                        resource_id: context.resource_id,
                        source_url: Some(path),
                        revision: context.revision,
                    },
                },
            });
        }
    }
    Ok(CatalogRead::Ready(resources))
}
fn contexts(
    source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &Connection,
    request: &Request,
) -> Result<Vec<ContextText>, Error> {
    match request.kind.as_str() {
        "paper" => {
            let snapshot =
                (source.latest)(db, &request.resource_id).map_err(|_| Error::Unavailable)?;
            match snapshot {
                None => Ok(vec![]),
                Some((id, hash)) => {
                    crate::ai::paper_context::agent_preview::materialize_submission_context(
                        source,
                        db,
                        &request.resource_id,
                        &id,
                        &hash,
                    )
                    .map_err(|_| Error::Unavailable)
                }
            }
        }
        "note" | "readingNote" => {
            let id = Uuid::parse_str(&request.resource_id).map_err(|_| Error::Invalid)?;
            let table = if request.kind == "note" {
                "note_pages"
            } else {
                "note_blocks"
            };
            let revision = db
                .query_row(
                    &format!("select revision from {table} where id=?1 and status='active'"),
                    [&request.resource_id],
                    |row| row.get(0),
                )
                .map_err(|_| Error::Unavailable)?;
            let context = if request.kind == "note" {
                crate::notes::agent_context::materialize_in(
                    db,
                    &crate::notes::agent_context::NoteContextRequest {
                        page_id: id,
                        expected_revision: revision,
                        block_id: None,
                    },
                )
            } else {
                crate::notes::reading_agent_context::materialize_in(
                    db,
                    &crate::notes::reading_agent_context::Request {
                        reading_note_id: id,
                        expected_revision: revision,
                    },
                )
            }
            .map_err(|_| Error::Unavailable)?;
            Ok(vec![context])
        }
        _ => Err(Error::Invalid),
    }
}
