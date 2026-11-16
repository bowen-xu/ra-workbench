select r.id, r.status
             from embedding_records r
             join embedding_index_generations g on g.id = r.index_generation_id
             where r.representation_id = ?1 and r.status != 'tombstoned'
             order by case
                 when g.status = 'building' and r.status in ('queued', 'running', 'failed', 'stale')
                 then 0 else 1 end,
                 case g.status
                 when 'active' then 0 when 'ready' then 1 when 'building' then 2 else 3 end,
                 g.generation desc
             limit 1
