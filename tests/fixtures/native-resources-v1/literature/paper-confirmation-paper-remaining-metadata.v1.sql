select count(*) from ai_paper_review_metadata_confirmations c
         left join ai_paper_review_metadata_confirmation_undos u on u.confirmation_id = c.id
         where c.batch_id = ?1 and u.id is null
