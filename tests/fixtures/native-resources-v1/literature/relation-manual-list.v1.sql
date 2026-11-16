select r.id from relation_assertions r
         where ?1 is null or r.project_id = ?1
         order by r.updated_at desc, r.id desc
