select count(*) from ai_paper_review_overview_batch_links l
         left join ai_overview_confirmation_undos u on u.batch_id = l.overview_batch_id
         where l.batch_id = ?1 and u.id is null
