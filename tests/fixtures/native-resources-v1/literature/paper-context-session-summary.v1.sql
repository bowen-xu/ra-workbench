select id, paper_id, context_snapshot_id, attachment_ids_json, parse_run_ids_json,
                    provider_id, model_id, reasoning_effort, bootstrap_kind,
                    source_workflow_id, source_workflow_revision, bootstrap_hash,
                    concept_retrieval_default, title, status, created_at, updated_at
             from paper_ai_reading_sessions where id = ?1