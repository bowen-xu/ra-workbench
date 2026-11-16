insert into embedding_generation_leases
             (id, index_generation_id, owner_kind, owner_id, expires_at, created_at)
             values (?1, ?2, 'query', ?3,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+5 minutes'),
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))