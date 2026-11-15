//! Writing scope checks run inside the shared queue transaction, including retries.
use super::*;
use crate::agent::repository::input_queue::{self, Error};
use crate::data_interface::{
    QueueEditFailure, QueueEditIntent, QueueEditOutcome, QueueEditScope, QueueEditStatus,
};
use research_agent_core::input_queue::{InputQueue, QueueEdit, QueueError};

impl WritingSubmissionService {
    pub async fn read_input_queue(
        &self,
        manuscript: Uuid,
        conversation: Uuid,
    ) -> Result<InputQueue, Error> {
        let store = self.store.clone();
        self.worker_retention
            .spawn(move || {
                let mut db = store.connection().map_err(|_| Error::Storage)?;
                input_queue::read_checked(&mut db, conversation, |db| {
                    binding(db, conversation, manuscript)
                })
            })
            .await
            .map_err(|_| Error::Storage)?
    }

    /// User editing cannot claim, release or acknowledge a model submission.
    #[cfg(test)]
    pub(crate) async fn edit_input_queue(
        &self,
        manuscript: Uuid,
        conversation: Uuid,
        request: Uuid,
        revision: u64,
        edit: QueueEdit,
    ) -> Result<InputQueue, QueueEditFailure> {
        let intent = QueueEditIntent {
            conversation_id: conversation,
            request_id: request,
            expected_revision: revision,
            expected_epoch: None,
            scope: QueueEditScope::Manuscript {
                manuscript_id: manuscript,
            },
            edit,
        };
        self.edit_input_queue_intent(intent).await
    }

    pub(crate) async fn edit_input_queue_intent(
        &self,
        intent: QueueEditIntent,
    ) -> Result<InputQueue, QueueEditFailure> {
        if matches!(
            intent.edit,
            QueueEdit::Prepare(_) | QueueEdit::Release(_) | QueueEdit::Admitted(_)
        ) || !matches!(intent.scope, QueueEditScope::Manuscript { .. })
        {
            return Err(QueueEditFailure::Domain(QueueError::Invalid.into()));
        }
        let database = self.store.shared_agent_connection();
        let original = intent.clone();
        self.worker_retention
            .spawn(move || {
                database
                    .edit_input_queue(&intent)
                    .map(QueueEditOutcome::into_queue)
            })
            .await
            .map_err(|_| crate::agent::queue_contract::worker_failure(&original))?
    }

    pub(crate) async fn read_input_queue_edit_status(
        &self,
        intent: QueueEditIntent,
    ) -> Result<QueueEditStatus, QueueEditFailure> {
        let database = self.store.shared_agent_connection();
        let original = intent.clone();
        self.worker_retention
            .spawn(move || database.input_queue_edit_status(&intent))
            .await
            .map_err(|_| crate::agent::queue_contract::worker_failure(&original))?
    }
}

fn binding(db: &Connection, conversation: Uuid, manuscript: Uuid) -> Result<(), Error> {
    check_binding(db, conversation, manuscript).map_err(|error| match error {
        WritingSubmissionError::Unbound => Error::Closed,
        _ => Error::Storage,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture;
    use super::*;
    use research_agent_core::input_queue::QueuedInput;

    #[tokio::test]
    async fn binding_is_required_for_read_write_and_replay_without_model_calls() {
        let f = fixture(true);
        let service = WritingSubmissionService::from_app_store(&f.store);
        let request = Uuid::new_v4();
        let item = QueuedInput {
            user_input: vec![],
            origin: None,
            id: Uuid::new_v4(),
            text: "queued fixture".into(),
        };
        assert!(
            service
                .read_input_queue(f.manuscript, f.conversation)
                .await
                .unwrap()
                .inputs()
                .is_empty()
        );
        let saved = service
            .edit_input_queue(
                f.manuscript,
                f.conversation,
                request,
                0,
                QueueEdit::Add(item.clone()),
            )
            .await
            .unwrap();
        assert_eq!(
            saved.inputs()[0].origin.as_ref().unwrap().kind,
            research_agent_core::conversation::BindingKind::Manuscript
        );
        assert_eq!(
            saved.inputs()[0].origin.as_ref().unwrap().resource_id,
            f.manuscript.to_string()
        );
        assert_eq!(
            service
                .edit_input_queue(
                    f.manuscript,
                    f.conversation,
                    request,
                    0,
                    QueueEdit::Add(item.clone())
                )
                .await
                .unwrap(),
            saved
        );
        assert!(matches!(
            service
                .read_input_queue(Uuid::new_v4(), f.conversation)
                .await,
            Err(Error::Closed)
        ));
        f.store
            .connection()
            .unwrap()
            .execute(
                "delete from agent_conversation_bindings where conversation_id=?1",
                [f.conversation.to_string()],
            )
            .unwrap();
        assert!(matches!(
            service
                .edit_input_queue(
                    f.manuscript,
                    f.conversation,
                    request,
                    0,
                    QueueEdit::Add(item)
                )
                .await,
            Err(QueueEditFailure::Domain(Error::Closed))
        ));
        assert_eq!(
            f.store
                .connection()
                .unwrap()
                .query_row("select count(*) from agent_submissions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn user_edit_cannot_acknowledge_execution_or_access_unbound_manuscript() {
        let f = fixture(false);
        let service = WritingSubmissionService::from_app_store(&f.store);
        assert!(matches!(
            service.read_input_queue(f.manuscript, f.conversation).await,
            Err(Error::Closed)
        ));
        for edit in [
            QueueEdit::Prepare(Uuid::new_v4()),
            QueueEdit::Release(Uuid::new_v4()),
            QueueEdit::Admitted(Uuid::new_v4()),
        ] {
            assert!(matches!(
                service
                    .edit_input_queue(f.manuscript, f.conversation, Uuid::new_v4(), 0, edit)
                    .await,
                Err(QueueEditFailure::Domain(Error::Queue(QueueError::Invalid)))
            ));
        }
    }
}
