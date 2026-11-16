update paper_analysis_turn_attempts
         set status = ?2, error_code = ?3, error_message = ?4,
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1