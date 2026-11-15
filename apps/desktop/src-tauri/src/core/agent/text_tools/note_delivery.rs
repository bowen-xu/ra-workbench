//! Notify the existing editor receipt consumer; never reopen a note writer.
use super::*;
use note::delivery::{self, Error};

pub(super) fn mark(value: &mut Value, call: &str) {
    value["noteDelivery"] = json!({"schemaVersion":1,"targetRequestId":format!("{call}:notes")});
}
impl TextTools {
    pub(super) fn finish_note_delivery(
        &self,
        call: &str,
        mut value: Value,
        targets: &[TargetResult],
        guard: Option<editor_barrier::Guard>,
    ) -> Value {
        let Some(intent) = value.get("noteDelivery") else {
            if let Some(guard) = guard {
                guard.complete(vec![]);
            }
            return value;
        };
        if targets
            .iter()
            .any(|target| target.status == TargetStatus::Unknown)
        {
            return value;
        }
        let expected = json!({"schemaVersion":1,"targetRequestId":format!("{call}:notes")});
        let outcome = if intent != &expected {
            Err(Error::Invalid)
        } else {
            (|| {
                let db = self.store.connection().map_err(Error::Owner)?;
                delivery::read(&db, &self.record, &format!("{call}:notes"))?.ok_or(Error::Missing)
            })()
        };
        let outcome = outcome.and_then(|delivery| {
            match guard {
                Some(guard) => guard.try_complete_review(delivery.pages, delivery.reviews),
                None => self
                    .store
                    .text_editors()
                    .publish_review_receipt(delivery.pages, delivery.reviews),
            }
            .map_err(|_| Error::Publish)
        });
        if let Err(error) = outcome {
            let detail = delivery::diagnostic(&self.record, call, &error);
            patch_journal::report(&detail);
            value = patch_journal::unknown(value, detail);
            match self.store.connection() {
                Ok(db) => {
                    if let Err(error) =
                        journal::record_sql(&db, self.record.conversation_id, call, &value, false)
                    {
                        patch_journal::report(&patch_journal::call_diagnostic(
                            &self.record,
                            call,
                            "call-recovery",
                            &error,
                        ));
                    }
                }
                Err(error) => patch_journal::report(&patch_journal::owner_diagnostic(
                    &self.record,
                    call,
                    "call-recovery",
                    &error,
                )),
            }
        }
        value
    }
}
