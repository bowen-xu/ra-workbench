select id, task_kind, status, provider_id, model_id, pipeline_version,
                     prompt_version, output_schema_version, user_goal, scope_hash,
                     input_item_count, max_input_tokens, max_output_tokens, timeout_ms,
                     cloud_consent_version, cloud_consented_at, cancel_requested, input_tokens,
                     output_tokens, cache_hit_tokens, cache_miss_tokens, error_code, error_message,
                     created_at, started_at, finished_at, updated_at from ai_runs order by id
