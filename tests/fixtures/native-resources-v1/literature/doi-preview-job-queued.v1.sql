update import_jobs set state = 'queued',
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1