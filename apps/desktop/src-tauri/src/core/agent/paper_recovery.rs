//! Bounded discovery of paper submissions prepared but never started.
//! A candidate is only a recovery prompt; restoring and starting still recheck
//! the frozen paper binding and submission identity.
use super::reading_submission;
use crate::storage::AppStore;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub conversation_id: Uuid,
    pub paper_id: String,
    pub after: Option<Uuid>,
    pub limit: u8,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub request_id: Uuid,
    pub turn_id: Uuid,
    pub paper_context_snapshot_id: String,
    pub message_excerpt: String,
    pub created_at: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub paper_id: String,
    pub items: Vec<Candidate>,
    pub next_cursor: Option<Uuid>,
}

#[tauri::command]
pub(crate) async fn agent_list_unstarted_paper_submissions(
    store: State<'_, AppStore>,
    request: Request,
) -> Result<Page, reading_submission::Error> {
    list(&store, request).await
}

pub(crate) async fn list(
    store: &AppStore,
    request: Request,
) -> Result<Page, reading_submission::Error> {
    validate(&request)?;
    let owned = store.clone();
    let paper_id = request.paper_id.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = owned
            .connection()
            .map_err(|_| reading_submission::Error::Unavailable)?;
        let tx = db
            .transaction()
            .map_err(|_| reading_submission::Error::Unavailable)?;
        reading_submission::check_binding(&tx, request.conversation_id, &paper_id)?;
        let mut statement = tx
            .prepare(
                "select s.request_id,s.turn_id,ps.paper_context_snapshot_id,substr(json_extract(s.input_json,'$.text'),1,200),s.created_at
             from agent_submissions s join agent_paper_submission_scopes ps on ps.conversation_id=s.conversation_id and ps.request_id=s.request_id
             where s.conversation_id=?1 and ps.paper_id=?2 and (?3 is null or s.request_id>?3)
             and not exists(select 1 from agent_items i where i.conversation_id=s.conversation_id
                 and json_extract(i.payload_json,'$.payload.type')='turnStarted'
                 and json_extract(i.payload_json,'$.payload.turnId')=s.turn_id)
             order by s.request_id limit ?4",
            )
            .map_err(|_| reading_submission::Error::Unavailable)?;
        let rows = statement
            .query_map(
                params![
                    request.conversation_id.to_string(),
                    paper_id,
                    request.after.map(|id| id.to_string()),
                    request.limit
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .map_err(|_| reading_submission::Error::Unavailable)?;
        let mut items = Vec::new();
        for row in rows {
            let (request_id, turn_id, snapshot, message_excerpt, created_at) =
                row.map_err(|_| reading_submission::Error::Unavailable)?;
            let request_id = Uuid::parse_str(&request_id)
                .map_err(|_| reading_submission::Error::Unavailable)?;
            let turn_id = Uuid::parse_str(&turn_id)
                .map_err(|_| reading_submission::Error::Unavailable)?;
            if request_id.is_nil()
                || turn_id.is_nil()
                || snapshot.trim().is_empty()
                || message_excerpt.trim().is_empty()
            {
                return Err(reading_submission::Error::Unavailable);
            }
            items.push(Candidate {
                request_id,
                turn_id,
                paper_context_snapshot_id: snapshot,
                message_excerpt,
                created_at,
            });
        }
        drop(statement);
        tx.commit()
            .map_err(|_| reading_submission::Error::Unavailable)?;
        let next_cursor = (items.len() == usize::from(request.limit))
            .then(|| items.last().map(|item| item.request_id))
            .flatten();
        Ok(Page {
            schema_version: 1,
            conversation_id: request.conversation_id,
            paper_id,
            items,
            next_cursor,
        })
    })
    .await
    .map_err(|_| reading_submission::Error::Unavailable)?
}

fn validate(request: &Request) -> Result<(), reading_submission::Error> {
    if request.conversation_id.is_nil()
        || request.paper_id.trim().is_empty()
        || request.paper_id.len() > 256
        || request.paper_id.contains('\0')
        || request.after.is_some_and(|id| id.is_nil())
        || !(1..=64).contains(&request.limit)
    {
        return Err(reading_submission::Error::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            conversation_id: Uuid::new_v4(),
            paper_id: "paper".into(),
            after: None,
            limit: 20,
        }
    }

    #[test]
    fn validation_rejects_unbounded_or_unscoped_requests() {
        assert!(validate(&request()).is_ok());
        let mut invalid = request();
        invalid.limit = 0;
        assert!(matches!(
            validate(&invalid),
            Err(reading_submission::Error::InvalidInput)
        ));
        let mut invalid = request();
        invalid.paper_id = "\0".into();
        assert!(matches!(
            validate(&invalid),
            Err(reading_submission::Error::InvalidInput)
        ));
        let mut invalid = request();
        invalid.after = Some(Uuid::nil());
        assert!(matches!(
            validate(&invalid),
            Err(reading_submission::Error::InvalidInput)
        ));
    }
}
