update import_results set state = 'failed', error_message = ?2,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1