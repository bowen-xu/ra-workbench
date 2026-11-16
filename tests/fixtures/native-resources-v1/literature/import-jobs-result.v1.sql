update import_results set state = ?2, item_id = ?3, error_message = ?4,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1