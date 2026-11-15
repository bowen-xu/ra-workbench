use super::*;
use crate::agent::{
    context_session::{self, PrepareRequest, resource_tools, sources::Resource},
    source_tools::OwnedSubmissionTools,
};
use research_agent_core::{
    conversation::ConversationId, model::ToolCall, protocol::ModelIdentity, runner::ToolOutput,
    tool_registry::OwnedToolRegistration,
};
use uuid::Uuid;

pub(super) struct ResultState {
    pub state: readback::State,
    pub output: Value,
}

pub(super) async fn submission(
    store: &AppStore,
    agents: &Arc<AgentSessionService>,
    seed: &fixture::Seed,
) -> Result<SubmissionRecord, Error> {
    let conversation = Uuid::new_v4();
    agents
        .create(ConversationId::from_uuid(conversation))
        .await?;
    let request = Uuid::new_v4();
    {
        let mut db = store.connection()?;
        let tx = db.transaction()?;
        let preview = context_session::materialize_in(
            &tx,
            store,
            PrepareRequest {
                conversation_id: conversation,
                request_id: request,
                text: "Read the explicitly shared first PDF page.".into(),
                user_input: vec![],
                resources: vec![Resource::Paper {
                    resource_id: seed.paper_id.clone(),
                    attachment_id: Some(seed.selected.id.clone()),
                    page_index: Some(0),
                }],
                retrieval: None,
                model: None,
                queue: None,
            },
            (
                ModelIdentity {
                    provider_id: "fixture-generation".into(),
                    model_id: "fixture-v1".into(),
                    revision: None,
                },
                None,
                None,
            ),
        )?;
        if preview.conversation_id != conversation || preview.request_id != request {
            return Err(
                "original source materialization returned another submission identity".into(),
            );
        }
        tx.commit()?;
    }
    Ok(SqliteSubmissionStore::for_app(store)
        .get(conversation, request)
        .await?)
}

async fn read(
    store: &AppStore,
    record: &SubmissionRecord,
    file: &str,
) -> Result<ToolOutput, Error> {
    let original = OwnedSubmissionTools::new(record.clone());
    let registration = resource_tools::registration(
        store.clone(),
        record.clone(),
        OwnedToolRegistration::new(original.definitions(), Arc::new(original)),
    );
    let (definitions, executor) = registration.into_parts();
    if definitions
        .iter()
        .filter(|tool| tool.name == "research_read_source")
        .count()
        != 1
    {
        return Err("production source tool registration is not unique".into());
    }
    let output = executor
        .authorize_and_execute(ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: Uuid::new_v4().to_string(),
            name: "research_read_source".into(),
            arguments: json!({"fileId":file,"view":"structured","maxChars":8000}),
        })
        .await?;
    Ok(output)
}

pub(super) async fn exercise(
    store: &AppStore,
    resources: &PaperContextResources,
    seed: &fixture::Seed,
    record: &SubmissionRecord,
) -> Result<ResultState, Error> {
    readback::verify_prefix(store, seed)?;
    let first = read(store, record, &seed.selected.id).await?;
    if first.is_error {
        return Err(format!("production structured read failed: {}", first.content).into());
    }
    let output: Value = serde_json::from_str(&first.content)?;
    let state = readback::capture(store, resources, seed, record, &output)?;
    let second = read(store, record, &seed.selected.id).await?;
    if second.is_error || serde_json::from_str::<Value>(&second.content)? != output {
        return Err("explicit repeated source tool read changed its returned evidence".into());
    }
    if readback::capture(store, resources, seed, record, &output)? != state {
        return Err("repeated source tool read changed its snapshot or original proof".into());
    }
    let rejected = read(store, record, &seed.unselected.id).await?;
    if !rejected.is_error {
        return Err("source tool exposed an ungranted attachment".into());
    }
    if readback::capture(store, resources, seed, record, &output)? != state {
        return Err("rejected ungranted read changed durable evidence".into());
    }
    Ok(ResultState { state, output })
}
