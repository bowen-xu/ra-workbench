select left_item_id, right_item_id from duplicate_candidates
             where id = ?1 and decision is null