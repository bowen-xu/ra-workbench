update import_results set state = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 where job_id = ?1 and state = 'pending'