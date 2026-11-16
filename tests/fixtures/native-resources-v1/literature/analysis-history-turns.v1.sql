select id, workflow_id, ordinal, turn_kind, status, candidate_id, created_at, finished_at
         from paper_analysis_turns where workflow_id = ?1 order by ordinal