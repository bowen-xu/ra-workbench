select coalesce(max(generation), 0) + 1 from embedding_index_generations
         where embedding_space_id = ?1 and namespace = ?2