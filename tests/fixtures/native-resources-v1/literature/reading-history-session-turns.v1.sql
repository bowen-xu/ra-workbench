select id, session_id, ordinal, user_message, assistant_message, messages_json, provider_id,
                model_id, reasoning_effort, context_window_tokens, context_window_source,
                status, input_hash, output_hash, input_tokens, output_tokens,
                error_code, error_message, concept_retrieval_enabled,
                concept_retrieval_trace_id, created_at, finished_at
         from paper_ai_reading_turns where session_id = ?1 order by ordinal
