update embedding_index_generations
             set status = 'active', activated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             where id = ?1 and status = 'building'