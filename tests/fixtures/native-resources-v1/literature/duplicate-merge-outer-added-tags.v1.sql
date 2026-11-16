select tag_id from item_tags source where source.item_id = ?1
         and not exists(select 1 from item_tags target where target.item_id = ?2 and target.tag_id = source.tag_id)