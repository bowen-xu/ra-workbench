select attempt.id, attempt.turn_id, attempt.attempt, attempt.status,
                attempt.normalized_messages_json, attempt.provider_id, attempt.model_id,
                attempt.reasoning_effort, attempt.ai_run_id, attempt.candidate_id,
                attempt.input_hash, attempt.output_hash, attempt.input_tokens,
                attempt.output_tokens, attempt.error_code, attempt.error_message,
                attempt.created_at, attempt.started_at, attempt.finished_at,
                recovery.recovery_reason, recovery.recovery_version,
                recovery.recovered_at, shared.conversation_id, shared.request_id, shared.turn_id,
                shared.ai_run_id, shared.candidate_id, shared.terminal_status
         from paper_analysis_turn_attempts attempt
         left join ai_candidate_recovery_audits recovery
            on recovery.candidate_id = attempt.candidate_id
         left join agent_paper_analysis_submissions shared
            on shared.attempt_id = attempt.id
         where attempt.turn_id = ?1 order by attempt.attempt