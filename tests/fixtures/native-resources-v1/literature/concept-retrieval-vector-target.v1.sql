select s.id, g.id, g.relative_database_path, s.dimension, s.model_id
             from embedding_index_generations g
             join embedding_spaces s on s.id = g.embedding_space_id
             where g.namespace = 'accepted_concepts' and g.status = 'active'
               and s.retired_at is null and s.provider_id = 'qwen-embedding'
               and s.model_id = ?1
               and exists (
                   select 1 from embedding_records r
                   where r.index_generation_id = g.id and r.status = 'ready'
               )
             order by g.activated_at desc, g.generation desc limit 1