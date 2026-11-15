//! Register the versioned text-tool contract over the original native owner.
use super::*;

pub(crate) fn registration(
    store: AppStore,
    record: SubmissionRecord,
    previous: OwnedToolRegistration,
) -> OwnedToolRegistration {
    registration_inner(
        store,
        record,
        previous,
        #[cfg(target_os = "macos")]
        None,
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn registration_with_workspace(
    store: AppStore,
    record: SubmissionRecord,
    previous: OwnedToolRegistration,
    workspace: file_workspace::FileWorkspaceHandle,
) -> OwnedToolRegistration {
    registration_inner(store, record, previous, workspace)
}

fn registration_inner(
    store: AppStore,
    record: SubmissionRecord,
    previous: OwnedToolRegistration,
    #[cfg(target_os = "macos")] file_workspace: file_workspace::FileWorkspaceHandle,
) -> OwnedToolRegistration {
    // Pre-migration stores are retained by historical host fixtures. Production
    // opens migrate first; never install an owner whose durable tables are absent.
    let migrated = store.connection().ok().and_then(|db| db.query_row("select exists(select 1 from sqlite_master where type='table' and name='agent_text_tool_contracts')", [], |row| row.get::<_,bool>(0)).ok());
    if migrated == Some(false) {
        return previous;
    }
    let version=store.connection().ok().and_then(|db|{
        let present:bool=db.query_row("select exists(select 1 from sqlite_master where type='table' and name='agent_text_tool_contracts')",[],|row|row.get(0)).ok()?;
        if !present{return Some(1);}
        db.execute("insert into agent_text_tool_contracts(conversation_id,request_id,version) values(?1,?2,2) on conflict do nothing",rusqlite::params![record.conversation_id.to_string(),record.request_id.to_string()]).ok()?;
        db.query_row("select version from agent_text_tool_contracts where conversation_id=?1 and request_id=?2",rusqlite::params![record.conversation_id.to_string(),record.request_id.to_string()],|row|row.get::<_,i64>(0)).ok()
    });
    let (old, executor) = previous.into_parts();
    let names = old
        .iter()
        .filter(|definition| definition.namespace.is_none())
        .map(|definition| definition.name.clone())
        .collect();
    let qualified_names = old
        .iter()
        .filter_map(|definition| {
            definition
                .namespace
                .as_ref()
                .map(|namespace| (namespace.clone(), definition.name.clone()))
        })
        .collect();
    let tools = Arc::new(TextTools {
        store,
        record,
        previous: executor,
        previous_names: names,
        previous_qualified_names: qualified_names,
        delivery_attempted: Default::default(),
        delivery_sealed: Default::default(),
        delivery_lease: Default::default(),
        cancellation: Default::default(),
        #[cfg(target_os = "macos")]
        file_workspace,
    });
    let definitions = if version == Some(1) {
        old
    } else {
        let mut definitions = old
            .into_iter()
            .filter(|definition| {
                definition.namespace.is_some()
                    || (definition.name != PATCH
                        && definition.name != READ
                        && definition.name != "research_propose_note_rewrite")
            })
            .collect::<Vec<_>>();
        if version == Some(2) {
            definitions.extend(tools.definitions());
        }
        definitions
    };
    OwnedToolRegistration::new(definitions, tools)
}
