select id from derived_research_views where project_id = ?1
         order by updated_at desc, id
