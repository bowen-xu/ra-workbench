select id from structured_reading_units
             where reverted_at is null
               and (?1 is null or project_id = ?1) and (?2 is null or paper_id = ?2)
             order by updated_at desc, id desc