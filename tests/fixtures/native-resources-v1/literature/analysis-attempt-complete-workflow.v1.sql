update paper_analysis_workflows
         set status = ?2, active_turn_kind = null, revision = revision + 1,
             completed_at = case when ?2 = 'needs_review'
                 then strftime('%Y-%m-%dT%H:%M:%fZ', 'now') else null end,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1