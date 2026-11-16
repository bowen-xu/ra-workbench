delete from tags where not exists
            (select 1 from item_tags where tag_id = tags.id)