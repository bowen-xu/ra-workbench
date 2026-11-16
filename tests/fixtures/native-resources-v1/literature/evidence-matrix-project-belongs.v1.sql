select exists(select 1 from project_items where project_id = ?1 and item_id = ?2)
