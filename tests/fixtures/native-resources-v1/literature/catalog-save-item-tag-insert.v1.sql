insert into item_tags (item_id, tag_id)
             select ?1, id from tags where name = ?2 collate nocase