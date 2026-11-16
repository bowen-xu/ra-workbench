update merge_operations set state = 'undone',
             undone_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1