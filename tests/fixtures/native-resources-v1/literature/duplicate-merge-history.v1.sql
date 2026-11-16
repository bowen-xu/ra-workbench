select m.id, m.survivor_item_id, survivor.title, m.merged_item_id, merged.title,
                m.state, m.created_at
         from merge_operations m
         join bibliographic_items survivor on survivor.id = m.survivor_item_id
         join bibliographic_items merged on merged.id = m.merged_item_id
         where m.state = 'applied'
           and survivor.trashed_at is null and merged.trashed_at is null
         order by m.created_at desc