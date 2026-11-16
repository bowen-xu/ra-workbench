insert into duplicate_candidates (id, left_item_id, right_item_id, reason, confidence, created_at)
         select ?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where not exists (
             select 1 from duplicate_candidates
             where decision is null and ((left_item_id = ?2 and right_item_id = ?3) or (left_item_id = ?3 and right_item_id = ?2))
         )