update metadata_candidates set state = 'rejected',
             decided_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and state = 'pending'