select workflow.id, paper_turn.id, attempt.id, attempt.status,
                attempt.error_code, attempt.error_message
         from paper_analysis_workflows workflow
         join paper_analysis_turns paper_turn on paper_turn.workflow_id = workflow.id
         join paper_analysis_turn_attempts attempt on attempt.turn_id = paper_turn.id
         where workflow.context_snapshot_id = ?1
           and workflow.provider_id = ?2 and workflow.model_id = ?3
           and (workflow.status = ?4 or (?6 = 'validating' and workflow.status = ?7))
           and paper_turn.turn_kind = ?5
           and paper_turn.status in ('running', 'failed', 'interrupted')
           and attempt.id = (
               select latest.id from paper_analysis_turn_attempts latest
               where latest.turn_id = paper_turn.id
               order by latest.attempt desc limit 1
           )
           and attempt.status in ('running', 'failed', 'interrupted')
           and attempt.candidate_id is null
           and (attempt.error_code = 'storage_error' or ?6 = 'validating')
         order by workflow.created_at, workflow.id