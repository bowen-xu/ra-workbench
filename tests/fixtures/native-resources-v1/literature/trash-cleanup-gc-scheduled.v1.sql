select exists(
               select 1 from content_object_gc_candidates g
               where g.content_sha256 = ?1
                 and not exists(
                   select 1 from content_object_references r
                   where r.content_sha256 = g.content_sha256
                 )
             )