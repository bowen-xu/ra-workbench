select s.id, s.provider_id, s.model_id, s.model_revision, s.dimension,
                g.generation, g.status, g.namespace,
                sum(case when r.status = 'ready' then 1 else 0 end),
                sum(case when r.status in ('queued', 'running') then 1 else 0 end),
                sum(case when r.status = 'failed' then 1 else 0 end),
                sum(case when r.status = 'stale' then 1 else 0 end)
         from embedding_index_generations g
         join embedding_spaces s on s.id = g.embedding_space_id
         left join embedding_records r on r.index_generation_id = g.id
         where g.status in ('building', 'ready', 'active')
         group by s.id, g.id
         order by g.namespace, s.created_at desc, g.generation desc