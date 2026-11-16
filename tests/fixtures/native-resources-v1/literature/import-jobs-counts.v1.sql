select coalesce(sum(state = 'imported'), 0), coalesce(sum(state = 'failed'), 0)
         from import_results where job_id = ?1