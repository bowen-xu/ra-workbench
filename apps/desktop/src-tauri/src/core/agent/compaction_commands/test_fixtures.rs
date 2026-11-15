//! Explicit test-only capture of actual native facts; ordinary tests write none.
use crate::agent::service::AgentSessionService;
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, ApplyResult, ConversationProjection},
};
use uuid::Uuid;

pub(crate) fn export(name: &str, events: &[AgentEvent]) {
    let Some(directory) = std::env::var_os("RA_COMPACT_FIXTURE_DIR") else {
        return;
    };
    assert!(matches!(
        name,
        "completed" | "failed" | "canceled" | "post-failed" | "post-canceled"
    ));
    let mut state = ConversationProjection::new(events.first().unwrap().conversation_id());
    for event in events {
        assert_eq!(state.apply(event).unwrap(), ApplyResult::Applied);
    }
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let target = directory.join(format!("{name}.json"));
    let temporary = directory.join(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let mut encoded = serde_json::to_string_pretty(events).unwrap();
    encoded.push('\n');
    std::fs::write(&temporary, encoded).unwrap();
    std::fs::rename(temporary, target).unwrap();
}

pub(crate) async fn export_service(name: &str, service: &AgentSessionService, id: Uuid) {
    if std::env::var_os("RA_COMPACT_FIXTURE_DIR").is_none() {
        return;
    }
    let mut events = vec![];
    let mut offset = 0;
    loop {
        let page = service
            .read_since(ConversationId::from_uuid(id), offset, 500)
            .await
            .unwrap();
        events.extend(page.items);
        match page.next_offset {
            Some(next) => offset = next,
            None => {
                assert_eq!(events.len() as u64, page.durable_count);
                break;
            }
        }
    }
    export(name, &events);
}
