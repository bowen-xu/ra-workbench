delete from creators where not exists
            (select 1 from item_creators where creator_id = creators.id)