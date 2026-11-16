update import_jobs set state = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 where id = ?1