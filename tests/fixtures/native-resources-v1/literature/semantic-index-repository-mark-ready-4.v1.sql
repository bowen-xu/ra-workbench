select g.id, g.namespace, g.embedding_space_id, g.status,
                (select count(*) from embedding_records pending
                 where pending.index_generation_id = g.id
                   and pending.status not in ('ready', 'tombstoned'))
         from embedding_index_generations g
         where g.id = (select index_generation_id from embedding_records where id = ?1)