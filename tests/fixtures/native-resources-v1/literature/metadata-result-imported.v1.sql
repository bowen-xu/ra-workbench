update import_results set state = 'imported', item_id = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1