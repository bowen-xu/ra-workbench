update duplicate_candidates set decision = ?2, decided_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and decision is null