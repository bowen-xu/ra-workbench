select g.id, g.namespace, g.relative_database_path, s.dimension
         from embedding_index_generations g
         join embedding_spaces s on s.id = g.embedding_space_id
         where g.status in ('building', 'ready', 'active')
         order by g.namespace, g.generation