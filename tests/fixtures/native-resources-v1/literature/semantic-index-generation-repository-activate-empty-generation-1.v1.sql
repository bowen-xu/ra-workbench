update embedding_index_generations
         set status = 'superseded', superseded_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where namespace = ?1 and embedding_space_id = ?2 and status = 'active'