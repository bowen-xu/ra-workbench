select id from structured_reading_units
             where paper_id = ?1 and unit_kind = ?2 and reverted_at is null
               and id not in (select reading_unit_id from reading_unit_template_fields)
               and ((?3 is null and project_id is null) or project_id = ?3)
             order by updated_at desc, id desc limit 1
