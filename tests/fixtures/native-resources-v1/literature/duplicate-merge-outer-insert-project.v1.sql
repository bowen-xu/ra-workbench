insert into project_items (project_id, item_id, added_at)
             select project_id, ?2, added_at from project_items where project_id = ?1 and item_id = ?3