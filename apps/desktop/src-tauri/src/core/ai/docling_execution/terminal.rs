//! Original stage completion order around the selected candidate publication.
use super::{native::*, *};
use crate::ai::{
    docling_generation_support::StageCompletion,
    generation_support::{AiRun, hash_text},
    run_repository, run_state_repository,
};
use std::time::Instant;

impl NativeDoclingHost {
    pub(super) async fn finish_success(
        &self,
        state: PreparedOverviewRunState,
        response: PaperGenerationResponse,
    ) -> Result<AiRun, AiError> {
        let PaperGenerationResponse {
            content,
            input_tokens,
            output_tokens,
        } = response;
        let stage = state.generation_stage_id;
        let hash = hash_text(&content);
        let started = state.generation_started;
        journal(&self.store, move |db| {
            run_state_repository::complete_stage(
                db,
                &stage,
                StageCompletion {
                    output_hash: Some(&hash),
                    input_tokens,
                    output_tokens,
                    duration_ms: elapsed_ms(started),
                },
            )
        })
        .await?;
        let validation_started = Instant::now();
        let run = state.run.id.clone();
        let hash = hash_text(&content);
        let validation_stage = journal(&self.store, move |db| {
            run_state_repository::begin_stage(db, &run, "validating", Some(&hash))
        })
        .await?;
        let candidate = self
            .service
            .save_analysis_candidate(
                self.store.clone(),
                self.key.clone(),
                state.run.id.clone(),
                state.bundle,
                content,
            )
            .await;
        match candidate {
            Ok(candidate) => {
                let output = candidate.payload_hash().to_owned();
                journal(&self.store, move |db| {
                    run_state_repository::complete_stage(
                        db,
                        &validation_stage,
                        StageCompletion {
                            output_hash: Some(&output),
                            input_tokens: None,
                            output_tokens: None,
                            duration_ms: elapsed_ms(validation_started),
                        },
                    )
                })
                .await?;
                let run = state.run.id.clone();
                journal(&self.store, move |db| {
                    run_state_repository::finish_run_guarded(
                        db,
                        &run,
                        "needs_review",
                        None,
                        None,
                        Some((input_tokens.unwrap_or(0), output_tokens.unwrap_or(0), 0, 0)),
                    )
                })
                .await?;
                journal(&self.store, move |db| {
                    run_repository::get_run(db, &state.run.id)
                })
                .await
            }
            Err(error) if outcome_unknown(&error) => Err(error),
            Err(error) => {
                let code = error.code();
                let _ = journal(&self.store, move |db| {
                    run_state_repository::fail_stage(
                        db,
                        &validation_stage,
                        code,
                        "结构化阅读候选校验失败；未保存 provider 原始响应",
                        elapsed_ms(validation_started),
                    )
                })
                .await;
                finish_failed(&self.store, state.run.id, &error).await?;
                Err(error)
            }
        }
    }
}
