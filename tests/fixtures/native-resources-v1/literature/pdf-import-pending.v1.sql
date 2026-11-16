select id, raw_input from import_results
                 where job_id = ?1 and state = 'pending' order by created_at, rowid