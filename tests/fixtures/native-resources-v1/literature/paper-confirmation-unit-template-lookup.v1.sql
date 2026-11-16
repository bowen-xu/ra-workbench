select u.id from structured_reading_units u join reading_unit_template_fields t on t.reading_unit_id = u.id
            where u.paper_id = ?1 and u.project_id is null and u.reverted_at is null and t.portrait_candidate_id = ?2 and t.field_id = ?3
            order by u.updated_at desc, u.id desc limit 1
