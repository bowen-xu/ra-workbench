update concept_algorithm_runs
         set status = 'failed', error_code = 'algorithm_failed',
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and status = 'running'
