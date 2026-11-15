//! Historical execution evidence is independent of current provider credentials.
use super::*;
use crate::{
    agent::{
        compaction_commands,
        repository::writing_start_inspection::{Fact, Intent},
        writing_commands::SubmissionPreview,
    },
    data_interface::{SqlImmediateDiagnostic, WritingStartInspection},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartStatusResponse<T> {
    schema_version: u8,
    intent: T,
    status: Status,
}
#[derive(Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Status {
    Started { result: StartResponse },
    Prepared { preview: Box<SubmissionPreview> },
    NotFound,
    Unconfirmed { detail: Box<SqlImmediateDiagnostic> },
}
fn valid(identity: &Intent) -> bool {
    ![
        identity.conversation_id,
        identity.manuscript_id,
        identity.request_id,
        identity.context_snapshot_id,
    ]
    .iter()
    .any(Uuid::is_nil)
        && identity
            .compact_revision
            .is_none_or(|revision| revision <= 9_007_199_254_740_991)
        && !identity.model.provider_id.trim().is_empty()
        && !identity.model.model_id.trim().is_empty()
}
async fn observe(service: &WritingSubmissionService, identity: Intent) -> Status {
    let c = identity.conversation_id;
    let r = identity.request_id;
    let m = identity.manuscript_id;
    match service.inspect_writing_start(identity).await {
        WritingStartInspection::Confirmed(Fact::Started { turn_id, outcome }) => Status::Started {
            result: StartResponse {
                schema_version: 1,
                conversation_id: c,
                request_id: r,
                turn_id,
                run_id: None,
                already_started: true,
                outcome,
            },
        },
        WritingStartInspection::Confirmed(Fact::Prepared(record)) => Status::Prepared {
            preview: Box::new(SubmissionPreview::from_record(*record, m)),
        },
        WritingStartInspection::Confirmed(Fact::Missing) => Status::NotFound,
        WritingStartInspection::Unconfirmed(detail) => Status::Unconfirmed { detail },
    }
}
pub(crate) async fn read_start_status(
    service: &WritingSubmissionService,
    request: StartRequest,
) -> Result<StartStatusResponse<StartRequest>, StartError> {
    let identity = Intent {
        conversation_id: request.conversation_id,
        manuscript_id: request.manuscript_id,
        request_id: request.request_id,
        context_snapshot_id: request.context_snapshot_id,
        model: request.model.clone(),
        compact_revision: None,
    };
    if !valid(&identity) {
        return Err(StartError::ConfirmationMismatch);
    }
    // Mode is echoed, not retrospectively proved. An observation never grants a
    // new execution or asks a retired model/profile to become available again.
    Ok(StartStatusResponse {
        schema_version: 1,
        status: observe(service, identity).await,
        intent: request,
    })
}
pub(crate) async fn read_compaction_start_status(
    service: &WritingSubmissionService,
    request: compaction_commands::StartRequest,
) -> Result<StartStatusResponse<compaction_commands::StartRequest>, compaction_commands::Error> {
    let compaction_commands::Host::Manuscript { manuscript_id } = &request.host else {
        return Err(compaction_commands::Error::InvalidInput);
    };
    let identity = Intent {
        conversation_id: request.conversation_id,
        manuscript_id: *manuscript_id,
        request_id: request.request_id,
        context_snapshot_id: request.context_snapshot_id,
        model: request.model.clone(),
        compact_revision: Some(request.expected_revision),
    };
    if !valid(&identity) {
        return Err(compaction_commands::Error::InvalidInput);
    }
    Ok(StartStatusResponse {
        schema_version: 1,
        status: observe(service, identity).await,
        intent: request,
    })
}
