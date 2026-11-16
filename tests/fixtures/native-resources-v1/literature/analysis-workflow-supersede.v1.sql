update paper_analysis_workflows
             set status = 'superseded', active_turn_kind = null,
                 revision = revision + 1, completed_at = coalesce(completed_at,
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             where id = ?1 and status not in ('superseded', 'canceled')