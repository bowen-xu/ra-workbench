//! Original fixed paper SQL and transaction owners.
use super::{ConfirmationPolicy, ConfirmationResources, mismatch, overview, settlement, undo};
use crate::ai::{confirmation_support::*, error::AiError, structured_resource_validation::Flow};
use research_workbench_native_sdk::paper_confirmation::wire::{
    PaperConfirmationDecisionV1 as Decision, PaperConfirmationInputV1 as Input,
};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

pub(crate) struct ConfirmationRepository<'db, 'ctx, P: ConfirmationPolicy> {
    pub(super) connection: &'db mut Connection,
    pub(super) resources: &'ctx ConfirmationResources,
    pub(super) policy: &'ctx mut P,
}
impl<'db, 'ctx, P: ConfirmationPolicy> ConfirmationRepository<'db, 'ctx, P> {
    pub(crate) fn new(
        connection: &'db mut Connection,
        resources: &'ctx ConfirmationResources,
        policy: &'ctx mut P,
    ) -> Self {
        Self {
            connection,
            resources,
            policy,
        }
    }
    pub(super) fn facts(&self) -> ConfirmationFacts<'_> {
        ConfirmationFacts::new(self.connection)
    }
    pub(super) fn overview_result(
        &self,
        candidate: &str,
        batch: &str,
    ) -> Result<AiOverviewConfirmationResult, AiError> {
        Ok(AiOverviewConfirmationResult {
            candidate: self.facts().candidate(candidate)?,
            batch: self.facts().batch(batch)?,
        })
    }
    pub(super) fn commit_fields(
        &mut self,
        plan: overview::Plan<'_>,
    ) -> Result<AiOverviewConfirmationResult, Flow<AiError, P::Pause>> {
        let tx = self.connection.transaction()?;
        let result = overview::apply(&tx, self.resources, self.policy, &plan);
        let settled = settlement::settle(tx, result, self.policy.explicit_settlement());
        settlement::finish(
            self.connection,
            settled,
            self.policy.explicit_settlement(),
            &plan.batch_id,
        )?;
        self.overview_result(&plan.candidate.id, &plan.batch_id)
            .map_err(Flow::original)
    }
    pub(super) fn commit_undo(
        &mut self,
        batch: &AiOverviewConfirmationBatch,
        key: &str,
    ) -> Result<AiOverviewConfirmationResult, Flow<AiError, P::Pause>> {
        let tx = self.connection.transaction()?;
        let result = undo::apply(&tx, self.resources, self.policy, batch, key);
        let settled = settlement::settle(tx, result, self.policy.explicit_settlement());
        settlement::finish(
            self.connection,
            settled,
            self.policy.explicit_settlement(),
            &batch.id,
        )?;
        self.overview_result(&batch.candidate_id, &batch.id)
            .map_err(Flow::original)
    }
    pub(super) fn reset_field(
        &mut self,
        batch: &str,
        field: &str,
        key: &str,
    ) -> Result<AiOverviewConfirmationResult, Flow<AiError, P::Pause>> {
        let tx = self.connection.transaction()?;
        let result = undo::apply_reset(&tx, self.policy, batch, field, key);
        let settled = settlement::settle(tx, result, self.policy.explicit_settlement());
        let candidate = settlement::finish(
            self.connection,
            settled,
            self.policy.explicit_settlement(),
            batch,
        )?;
        self.overview_result(&candidate, batch)
            .map_err(Flow::original)
    }
}

#[derive(Debug)]
pub(super) struct PendingMetadataUndo {
    pub id: String,
    pub field_name: String,
    pub previous_confirmed_by_user: bool,
    pub applied_updated_at: String,
}

impl<P: ConfirmationPolicy> ConfirmationRepository<'_, '_, P> {
    pub(super) fn confirm_metadata_field(
        &mut self,
        batch_id: &str,
        paper_id: &str,
        field_name: &str,
    ) -> Result<(), Flow<AiError, P::Pause>> {
        let exists = self.connection.query_row(
            self.resources
                .metadata_receipt_exists
                .trim_end_matches('\n'),
            params![batch_id, field_name],
            |row| row.get::<_, bool>(0),
        )?;
        if exists {
            return Ok(());
        }
        let source = self
            .connection
            .query_row(
                self.resources.metadata_source.trim_end_matches('\n'),
                params![paper_id, field_name],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let decision = self.policy.decide(Input::MetadataSource {
            field_name: field_name.into(),
            source: source.clone(),
        })?;
        let Decision::MetadataSource { apply } = decision else {
            return Err(mismatch());
        };
        if !apply {
            return Ok(());
        }
        let (previous_confirmed, previous_updated_at) =
            source.ok_or_else(|| AiError::TemporaryUnavailable)?;
        let transaction = self.connection.transaction()?;
        let result = (|| {
            let applied_updated_at = transaction.query_row(
                self.resources.metadata_clock.trim_end_matches('\n'),
                [],
                |row| row.get::<_, String>(0),
            )?;
            let changed = transaction.execute(
                self.resources.metadata_confirm.trim_end_matches('\n'),
                params![
                    paper_id,
                    field_name,
                    previous_updated_at,
                    applied_updated_at
                ],
            )?;
            if changed != 1 {
                return Err(
                    AiError::InvalidInput("字段已被其他操作修改，请重新载入".to_owned()).into(),
                );
            }
            transaction.execute(
                self.resources
                    .metadata_receipt_insert
                    .trim_end_matches('\n'),
                params![
                    Uuid::new_v4().to_string(),
                    batch_id,
                    field_name,
                    previous_confirmed,
                    previous_updated_at,
                    applied_updated_at
                ],
            )?;

            Ok(())
        })();
        let settled = settlement::settle(transaction, result, self.policy.explicit_settlement());
        settlement::finish(
            self.connection,
            settled,
            self.policy.explicit_settlement(),
            batch_id,
        )
    }
    pub(super) fn load_pending_metadata_undo(
        &self,
        batch_id: &str,
    ) -> Result<Vec<PendingMetadataUndo>, AiError> {
        Ok(self
            .connection
            .prepare(self.resources.metadata_pending_undo.trim_end_matches('\n'))?
            .query_map([batch_id], |row| {
                Ok(PendingMetadataUndo {
                    id: row.get(0)?,
                    field_name: row.get(1)?,
                    previous_confirmed_by_user: row.get(2)?,
                    applied_updated_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
    pub(super) fn undo_metadata_field(
        &mut self,
        batch: &AiPaperReviewConfirmationBatch,
        confirmation: &PendingMetadataUndo,
    ) -> Result<(), Flow<AiError, P::Pause>> {
        let transaction = self.connection.transaction()?;
        let result = (|| {
            let changed = transaction.execute(
                self.resources.metadata_restore.trim_end_matches('\n'),
                params![
                    batch.paper_id,
                    confirmation.field_name,
                    confirmation.applied_updated_at,
                    confirmation.previous_confirmed_by_user
                ],
            )?;
            if changed != 1 {
                return Err(
                    AiError::InvalidInput("字段确认后已被修改，拒绝覆盖新版本".to_owned()).into(),
                );
            }
            transaction.execute(
                self.resources.metadata_undo_insert.trim_end_matches('\n'),
                params![
                    Uuid::new_v4().to_string(),
                    confirmation.id,
                    format!(
                        "paper-review-metadata-undo:{}:{}",
                        batch.id, confirmation.id
                    )
                ],
            )?;

            Ok(())
        })();
        let settled = settlement::settle(transaction, result, self.policy.explicit_settlement());
        settlement::finish(
            self.connection,
            settled,
            self.policy.explicit_settlement(),
            &batch.id,
        )
    }
    pub(super) fn paper_batch(
        &self,
        batch_id: &str,
    ) -> Result<AiPaperReviewConfirmationBatch, AiError> {
        let mut batch = self
            .connection
            .query_row(
                self.resources.paper_batch.trim_end_matches('\n'),
                [batch_id],
                |row| {
                    let undone_at = row.get::<_, Option<String>>(5)?;
                    Ok(AiPaperReviewConfirmationBatch {
                        id: row.get(0)?,
                        paper_id: row.get(1)?,
                        project_id: row.get(2)?,
                        idempotency_key: row.get(3)?,
                        created_at: row.get(4)?,
                        metadata_confirmations: Vec::new(),
                        overview_batch_ids: Vec::new(),
                        active: undone_at.is_none(),
                        undone_at,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| AiError::InvalidInput("找不到文献确认批次".to_owned()))?;
        batch.metadata_confirmations = self
            .connection
            .prepare(self.resources.paper_batch_metadata.trim_end_matches('\n'))?
            .query_map([batch_id], |row| {
                let undone_at = row.get::<_, Option<String>>(3)?;
                Ok(AiPaperReviewMetadataConfirmation {
                    id: row.get(0)?,
                    field_name: row.get(1)?,
                    created_at: row.get(2)?,
                    active: undone_at.is_none(),
                    undone_at,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        batch.overview_batch_ids = self
            .connection
            .prepare(self.resources.paper_batch_overviews.trim_end_matches('\n'))?
            .query_map([batch_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(batch)
    }

    pub(super) fn paper_batch_for_key(
        &self,
        key: &str,
    ) -> Result<Option<(String, String, String)>, AiError> {
        Ok(self
            .connection
            .query_row(
                self.resources.paper_batch_for_key.trim_end_matches('\n'),
                [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?)
    }
    pub(super) fn paper_exists(&self, paper: &str) -> Result<bool, AiError> {
        Ok(self.connection.query_row(
            self.resources.paper_exists.trim_end_matches('\n'),
            [paper],
            |row| row.get(0),
        )?)
    }
    pub(super) fn insert_paper_batch(
        &self,
        id: &str,
        draft: &AiPaperReviewConfirmationDraft,
        request: &str,
    ) -> Result<(), AiError> {
        self.connection.execute(
            self.resources.paper_batch_insert.trim_end_matches('\n'),
            params![
                id,
                draft.paper_id,
                draft.project_id,
                request,
                draft.idempotency_key
            ],
        )?;
        Ok(())
    }
    pub(super) fn paper_undo_for_key(&self, key: &str) -> Result<Option<String>, AiError> {
        Ok(self
            .connection
            .query_row(
                self.resources.paper_undo_for_key.trim_end_matches('\n'),
                [key],
                |row| row.get(0),
            )
            .optional()?)
    }
    pub(super) fn remaining_metadata(&self, batch: &str) -> Result<i64, AiError> {
        Ok(self.connection.query_row(
            self.resources
                .paper_remaining_metadata
                .trim_end_matches('\n'),
            [batch],
            |row| row.get(0),
        )?)
    }
    pub(super) fn remaining_overviews(&self, batch: &str) -> Result<i64, AiError> {
        Ok(self.connection.query_row(
            self.resources
                .paper_remaining_overviews
                .trim_end_matches('\n'),
            [batch],
            |row| row.get(0),
        )?)
    }
    pub(super) fn append_paper_undo(
        &self,
        id: &str,
        batch: &str,
        key: &str,
    ) -> Result<(), AiError> {
        self.connection.execute(
            self.resources.paper_undo_insert.trim_end_matches('\n'),
            params![id, batch, key],
        )?;
        Ok(())
    }
    pub(super) fn link_overview(&self, batch: &str, overview: &str) -> Result<(), AiError> {
        self.connection.execute(
            self.resources.paper_overview_link.trim_end_matches('\n'),
            params![batch, overview],
        )?;
        Ok(())
    }
    pub(super) fn paper_result(
        &self,
        id: &str,
        module_failures: Vec<String>,
    ) -> Result<AiPaperReviewConfirmationResult, AiError> {
        let batch = self.paper_batch(id)?;
        let ids = self
            .connection
            .prepare(self.resources.paper_candidate_ids.trim_end_matches('\n'))?
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let candidates = ids
            .iter()
            .map(|id| self.facts().candidate(id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(AiPaperReviewConfirmationResult {
            batch,
            candidates,
            module_failures,
        })
    }
}
