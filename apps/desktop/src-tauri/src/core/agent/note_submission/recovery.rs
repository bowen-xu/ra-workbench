//! Bounded local discovery; a candidate is not an execution authorization.
use super::{Error, source::Resource};
use crate::storage::AppStore;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub conversation_id: Uuid,
    pub page_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u8,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub request_id: Uuid,
    pub turn_id: Uuid,
    pub message_excerpt: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<crate::agent::queue_contract::Confirmation>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub page_id: Uuid,
    pub items: Vec<Candidate>,
    pub next_cursor: Option<Uuid>,
}

pub(crate) struct SelectedRequest {
    pub conversation_id: Uuid,
    pub resource: Resource,
    pub after: Option<Uuid>,
    pub limit: u8,
}
pub(crate) struct Candidates {
    pub items: Vec<Candidate>,
    pub next_cursor: Option<Uuid>,
}

/// Read authorization and candidates from one SQLite snapshot. Identity order is
/// stable pagination, not chronological priority; a full page may precede an empty one.
pub(crate) async fn list(store: &AppStore, request: Request) -> Result<Page, Error> {
    let candidates = list_selected(
        store,
        SelectedRequest {
            conversation_id: request.conversation_id,
            resource: Resource::Page(request.page_id),
            after: request.after,
            limit: request.limit,
        },
    )
    .await?;
    Ok(Page {
        schema_version: 1,
        conversation_id: request.conversation_id,
        page_id: request.page_id,
        items: candidates.items,
        next_cursor: candidates.next_cursor,
    })
}

pub(crate) async fn list_selected(
    store: &AppStore,
    request: SelectedRequest,
) -> Result<Candidates, Error> {
    list_selected_kind(store, request, false).await
}
pub(crate) async fn list_queue_selected(
    store: &AppStore,
    request: SelectedRequest,
) -> Result<Candidates, Error> {
    list_selected_kind(store, request, true).await
}
async fn list_selected_kind(
    store: &AppStore,
    request: SelectedRequest,
    queued: bool,
) -> Result<Candidates, Error> {
    if request.conversation_id.is_nil()
        || request.resource.id().is_nil()
        || request.after.is_some_and(|id| id.is_nil())
        || !(1..=64).contains(&request.limit)
    {
        return Err(Error::InvalidInput);
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        request.resource.check_binding(&tx, request.conversation_id)?;
        let mut statement = tx.prepare(
            "select s.request_id,s.turn_id,substr(json_extract(s.input_json,'$.text'),1,200),s.created_at,a.input_id,a.expected_revision
             from agent_submissions s
             left join agent_queue_submission_origins q on q.conversation_id=s.conversation_id and q.request_id=s.request_id
             left join agent_input_queue_admissions a on a.conversation_id=s.conversation_id and a.request_id=s.request_id and a.input_id=q.input_id
             where s.conversation_id=?1 and (?3 is null or s.request_id>?3)
             and s.schema_version=1 and json_array_length(s.input_json,'$.context')=1
             and json_extract(s.input_json,'$.context[0].kind')='note'
             and json_extract(s.input_json,'$.context[0].resourceId')=?2
             and json_valid(json_extract(s.input_json,'$.context[0].anchor'))
             and json_extract(json_extract(s.input_json,'$.context[0].anchor'),?5)=?2
             and (?6 is null or json_extract(s.input_json,'$.context[0].sourceId')=?6)
             and ((?7=0 and q.input_id is null) or (?7=1 and a.input_id is not null))
             and not exists(select 1 from agent_items i where i.conversation_id=s.conversation_id
                 and json_extract(i.payload_json,'$.payload.type')='turnStarted'
                 and json_extract(i.payload_json,'$.payload.turnId')=s.turn_id)
             order by s.request_id limit ?4"
        ).map_err(|_| Error::Unavailable)?;
        let source_id = match request.resource {Resource::Page(_) => None, Resource::Reading(id) => Some(format!("reading-note:{id}"))};
        let rows = statement.query_map(params![request.conversation_id.to_string(),request.resource.id().to_string(),request.after.map(|id|id.to_string()),request.limit,format!("$.{}",request.resource.anchor_key()),source_id,queued], |row| {
            Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,Option<String>>(4)?,row.get::<_,Option<u64>>(5)?))
        }).map_err(|_| Error::Unavailable)?;
        let mut items = Vec::new();
        for row in rows {
            let (id, turn, message_excerpt, created_at, input_id, expected_revision) = row.map_err(|_| Error::Unavailable)?;
            let request_id = Uuid::parse_str(&id).map_err(|_| Error::Unavailable)?;
            let turn_id = Uuid::parse_str(&turn).map_err(|_| Error::Unavailable)?;
            if request_id.is_nil() || turn_id.is_nil() { return Err(Error::Unavailable); }
            let queue = match (queued,input_id,expected_revision) {
                (false,None,None) => None,
                (true,Some(id),Some(revision)) if revision < 9_007_199_254_740_991 => {
                    let input_id = Uuid::parse_str(&id).map_err(|_|Error::Unavailable)?;
                    if input_id.is_nil() {return Err(Error::Unavailable);}
                    Some(crate::agent::queue_contract::Confirmation {input_id,expected_revision:revision})
                }
                _ => return Err(Error::Unavailable),
            };
            items.push(Candidate {request_id,turn_id,message_excerpt,created_at,queue});
        }
        drop(statement);
        tx.commit().map_err(|_| Error::Unavailable)?;
        let next_cursor = (items.len()==usize::from(request.limit)).then(||items.last().map(|item|item.request_id)).flatten();
        Ok(Candidates {items,next_cursor})
    }).await.map_err(|_| Error::Unavailable)?
}
