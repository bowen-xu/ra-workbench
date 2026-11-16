update paper_analysis_turn_attempts
         set status = 'succeeded', ai_run_id = ?2, candidate_id = ?3, input_hash = ?4,
             output_hash = ?5, input_tokens = ?6, output_tokens = ?7,
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1