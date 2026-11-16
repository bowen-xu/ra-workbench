update paper_analysis_workflows
         set status = ?2, active_turn_kind = null, revision = revision + 1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1