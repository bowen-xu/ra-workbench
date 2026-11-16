update duplicate_candidates set decision = 'merge',
             decided_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1