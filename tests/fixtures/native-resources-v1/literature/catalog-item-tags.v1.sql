select t.name from item_tags it join tags t on t.id = it.tag_id
             where it.item_id = ?1 order by t.name collate nocase