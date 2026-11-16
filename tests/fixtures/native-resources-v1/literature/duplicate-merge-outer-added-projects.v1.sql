select project_id from project_items source where source.item_id = ?1
         and not exists(select 1 from project_items target where target.item_id = ?2 and target.project_id = source.project_id)