select id, title from bibliographic_items
         where id in (select value from json_each(?1)) order by id
