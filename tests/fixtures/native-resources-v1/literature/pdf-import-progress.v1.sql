update import_jobs set completed_count = completed_count + 1,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1