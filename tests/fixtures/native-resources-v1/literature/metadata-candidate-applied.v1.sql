update metadata_candidates set state = 'applied',
             decided_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1