select b.id from literature_search s
         join bibliographic_items b on b.id = s.item_id
         where literature_search match ?1 and b.trashed_at is not null order by rank