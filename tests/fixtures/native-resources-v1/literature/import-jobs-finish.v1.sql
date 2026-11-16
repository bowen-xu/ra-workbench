update import_jobs set state = ?2, completed_count = total_count,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1