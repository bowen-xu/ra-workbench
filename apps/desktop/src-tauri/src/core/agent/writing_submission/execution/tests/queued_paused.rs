//! Explicit queued confirmation resumes atomically; old receipts cannot undo a later pause.
use super::super::super::tests::{Fixture, managed_fixture, queue_preview_intent};
use super::*;
use research_agent_core::input_queue::{InputQueue, QueueEdit, QueuedInput};

struct PausedQueue {
    fixture: Fixture,
    service: WritingSubmissionService,
    prepared: PreparedWritingSubmission,
    input: Uuid,
    tail: Uuid,
    revision: u64,
    profile: WritingExecutionProfile,
    sessions: Arc<AgentSessionService>,
    transport: Arc<Transport>,
}
impl PausedQueue {
    async fn new() -> Self {
        let f = managed_fixture(true);
        let service = WritingSubmissionService::from_app_store(&f.store);
        let mut request = request(&f, Uuid::new_v4());
        request.model = Some(AgentModelSelection {
            provider_id: "deepseek".into(),
            model_id: "deepseek-v4-pro".into(),
            reasoning_effort: None,
            service_tier: None,
        });
        let input = Uuid::new_v4();
        service
            .edit_input_queue(
                f.manuscript,
                f.conversation,
                Uuid::new_v4(),
                0,
                QueueEdit::Add(QueuedInput {
                    id: input,
                    text: request.text.clone(),
                    user_input: vec![],
                    origin: None,
                }),
            )
            .await
            .unwrap();
        let receipt = WritingSubmissionReceipt {
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
            request_id: request.request_id,
        };
        service
            .prepare_queue_preview(&f.models, queue_preview_intent(&request, input, 1), request)
            .await
            .unwrap();
        let tail = Uuid::new_v4();
        service
            .edit_input_queue(
                f.manuscript,
                f.conversation,
                Uuid::new_v4(),
                2,
                QueueEdit::Add(QueuedInput {
                    id: tail,
                    text: "Held tail".into(),
                    user_input: vec![],
                    origin: None,
                }),
            )
            .await
            .unwrap();
        service
            .edit_input_queue(
                f.manuscript,
                f.conversation,
                Uuid::new_v4(),
                3,
                QueueEdit::PauseInput(tail),
            )
            .await
            .unwrap();
        let queue = service
            .edit_input_queue(
                f.manuscript,
                f.conversation,
                Uuid::new_v4(),
                4,
                QueueEdit::Pause,
            )
            .await
            .unwrap();
        let prepared = service.restore(&f.models, receipt).await.unwrap();
        let profile = crate::agent::execution_profile::execution_profile(
            &prepared.record.input.model,
            crate::agent::execution_profile::AgentExecutionMode::DeepSeekNonThinking,
        )
        .unwrap();
        let sessions = Arc::new(AgentSessionService::new(Arc::new(
            SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into())
                .unwrap(),
        )));
        Self {
            fixture: f,
            service,
            prepared,
            input,
            tail,
            revision: queue.revision(),
            profile,
            sessions,
            transport: Arc::new(Transport(AtomicUsize::new(0))),
        }
    }
    async fn validate(&self, revision: u64) -> Result<(), WritingStartError> {
        self.service
            .validate_start_for_queue(
                &self.prepared,
                &confirmation(&self.prepared.record, self.fixture.manuscript),
                &self.profile,
                Some((self.input, revision)),
            )
            .await
    }
    async fn queue(&self) -> InputQueue {
        self.service
            .read_input_queue(self.fixture.manuscript, self.fixture.conversation)
            .await
            .unwrap()
    }
    async fn edit(&self, edit: QueueEdit) -> InputQueue {
        let revision = self.queue().await.revision();
        self.service
            .edit_input_queue(
                self.fixture.manuscript,
                self.fixture.conversation,
                Uuid::new_v4(),
                revision,
                edit,
            )
            .await
            .unwrap()
    }
    async fn start(&self) -> Result<SubmissionAdmission, WritingStartError> {
        self.service
            .start_transport(
                self.sessions.clone(),
                self.prepared.record.clone(),
                self.profile.clone(),
                self.transport.clone(),
            )
            .await
    }
    async fn completed_once(&self) {
        let SubmissionAdmission::Started(owned) = self.start().await.unwrap() else {
            panic!("first native admission must start");
        };
        assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
        assert!(matches!(
            self.start().await.unwrap(),
            SubmissionAdmission::AlreadyStarted {
                outcome: Some(TurnOutcome::Completed),
                ..
            }
        ));
        assert_eq!(self.transport.0.load(Ordering::SeqCst), 1);
    }
    fn assert_tail(&self, queue: &InputQueue) {
        assert_eq!(queue.preparing(), None);
        assert_eq!(queue.inputs().len(), 1);
        assert_eq!(queue.inputs()[0].id, self.tail);
        assert!(queue.input_paused(self.tail));
        assert!(queue.next_input().is_none());
    }
}

#[tokio::test]
async fn current_paused_confirmation_atomically_consumes_only_head_and_resumes_once() {
    let f = PausedQueue::new().await;
    let before = f.queue().await;
    assert!(before.paused());
    assert!(f.validate(2).await.is_err());
    assert_eq!(f.queue().await, before);
    assert_eq!(f.transport.0.load(Ordering::SeqCst), 0);
    f.validate(f.revision).await.unwrap();
    let admitted = f.queue().await;
    assert!(!admitted.paused());
    assert_eq!(admitted.revision(), f.revision + 2);
    f.assert_tail(&admitted);
    f.completed_once().await;
    f.validate(f.revision).await.unwrap();
    assert_eq!(f.queue().await, admitted);
}

#[tokio::test]
async fn original_admission_does_not_resume_a_later_pause_and_explicit_resume_starts_once() {
    let f = PausedQueue::new().await;
    f.validate(f.revision).await.unwrap();
    let paused = f.edit(QueueEdit::Pause).await;
    f.validate(f.revision).await.unwrap();
    assert_eq!(f.queue().await, paused);
    assert!(f.start().await.is_err());
    assert_eq!(f.transport.0.load(Ordering::SeqCst), 0);
    let conversation = ConversationId::from_uuid(f.fixture.conversation);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while f.sessions.tasks().has_active(conversation).unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(f.sessions.open(conversation).await.unwrap().seq(), 0);
    assert_eq!(f.queue().await, paused);
    let resumed = f.edit(QueueEdit::Resume).await;
    f.validate(f.revision).await.unwrap();
    assert_eq!(f.queue().await, resumed);
    f.assert_tail(&resumed);
    f.completed_once().await;
}

#[tokio::test]
async fn legacy_paused_admission_requires_explicit_resume_without_replacing_its_receipt() {
    let f = PausedQueue::new().await;
    let legacy = {
        let mut db = f.fixture.store.connection().unwrap();
        crate::agent::repository::input_queue::admit_checked(
            &mut db,
            f.fixture.conversation,
            f.prepared.record.request_id,
            f.revision,
            f.input,
            f.prepared.record.input.clone(),
            |_| Ok(()),
        )
        .unwrap()
        .1
    };
    assert!(legacy.paused());
    assert_eq!(legacy.revision(), f.revision + 1);
    f.assert_tail(&legacy);
    // Receipt replay cannot turn the earlier partial admission into a fresh
    // confirmation or remove the existing journal pause barrier.
    f.validate(f.revision).await.unwrap();
    assert_eq!(f.queue().await, legacy);
    assert!(f.start().await.is_err());
    assert_eq!(f.transport.0.load(Ordering::SeqCst), 0);
    let conversation = ConversationId::from_uuid(f.fixture.conversation);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while f.sessions.tasks().has_active(conversation).unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(f.sessions.open(conversation).await.unwrap().seq(), 0);
    let resumed = f.edit(QueueEdit::Resume).await;
    f.validate(f.revision).await.unwrap();
    assert_eq!(f.queue().await, resumed);
    f.assert_tail(&resumed);
    f.completed_once().await;
}
