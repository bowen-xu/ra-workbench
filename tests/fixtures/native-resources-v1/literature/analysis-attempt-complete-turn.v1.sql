update paper_analysis_turns set status = 'succeeded', candidate_id = ?2,
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1