select id from embedding_index_generations
             where embedding_space_id = ?1 and namespace = ?2 and status = 'building'