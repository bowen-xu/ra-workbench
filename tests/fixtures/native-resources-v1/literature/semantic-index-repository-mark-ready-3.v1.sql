update embedding_index_generations
         set lance_dataset_version = ?2,
             member_count = (select count(*) from embedding_records
                             where index_generation_id = ?1 and status = 'ready')
         where id = (select index_generation_id from embedding_records where id = ?1)