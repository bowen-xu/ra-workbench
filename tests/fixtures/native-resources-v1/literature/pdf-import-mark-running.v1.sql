update import_jobs set state = 'running', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             where id = ?1 and state = 'queued'