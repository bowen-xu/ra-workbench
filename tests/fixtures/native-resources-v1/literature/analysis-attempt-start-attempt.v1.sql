insert into paper_analysis_turn_attempts
         (id, turn_id, attempt, status, normalized_messages_json, provider_id, model_id,
          reasoning_effort, created_at, started_at)
         values (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))