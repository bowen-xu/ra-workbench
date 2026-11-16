select j.id
             from embedding_records r
             join embedding_outbox_jobs j on j.embedding_record_id = r.id
             where r.representation_id = ?1 and r.embedding_space_id = ?2
               and r.index_generation_id = ?3
             order by j.attempt desc limit 1