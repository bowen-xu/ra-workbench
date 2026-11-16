update concept_algorithm_runs
         set status = 'succeeded', result_count = ?2,
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status = 'running'
