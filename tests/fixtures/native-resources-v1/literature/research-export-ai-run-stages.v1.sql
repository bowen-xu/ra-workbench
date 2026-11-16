select id, run_id, stage_kind, attempt, status, input_hash, output_hash,
                     input_tokens, output_tokens, duration_ms, error_code, error_message,
                     started_at, finished_at, created_at from ai_run_stages
              order by run_id, created_at, attempt
