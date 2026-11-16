select paper_turn.id, attempt.id
             from paper_analysis_turns paper_turn
             join paper_analysis_turn_attempts attempt on attempt.turn_id = paper_turn.id
             where paper_turn.workflow_id = ?1 and paper_turn.turn_kind = ?2
               and paper_turn.status = 'running' and attempt.status = 'running'
             order by attempt.attempt desc limit 1