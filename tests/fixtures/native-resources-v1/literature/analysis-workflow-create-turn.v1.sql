insert into paper_analysis_turns
             (id, workflow_id, ordinal, turn_kind, status, created_at)
             values (?1, ?2, ?3, ?4, 'queued', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))