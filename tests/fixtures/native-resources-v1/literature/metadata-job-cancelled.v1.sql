update import_jobs set state = 'cancelled', cancellation_requested = 1,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             where id = (select job_id from import_results where id = ?1)