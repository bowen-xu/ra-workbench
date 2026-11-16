select id from paper_analysis_workflows
             where paper_id = ?1 and status != 'superseded'
             order by created_at desc, id desc limit 1