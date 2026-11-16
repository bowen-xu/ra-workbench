select d.id, d.left_item_id, l.title, d.right_item_id, r.title, d.reason, d.confidence
         from duplicate_candidates d
         join bibliographic_items l on l.id = d.left_item_id
         join bibliographic_items r on r.id = d.right_item_id
         where d.decision is null
           and l.archived_at is null and l.trashed_at is null
           and r.archived_at is null and r.trashed_at is null
         order by d.confidence desc, d.created_at