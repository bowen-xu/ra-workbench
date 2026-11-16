select count(*) from bibliographic_items where id in (select value from json_each(?1))
