update import_jobs set cancellation_requested = 1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and state in ('queued', 'running')