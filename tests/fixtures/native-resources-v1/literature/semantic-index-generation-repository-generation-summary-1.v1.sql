select g.id, g.embedding_space_id, g.namespace, g.generation, g.status,
                    count(r.id), sum(case when r.status = 'ready' then 1 else 0 end),
                    g.created_at, g.activated_at
             from embedding_index_generations g
             left join embedding_records r on r.index_generation_id = g.id
             where g.id = ?1 group by g.id