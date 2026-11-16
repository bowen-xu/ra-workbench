select c.id, c.field_name, c.previous_confirmed_by_user, c.applied_updated_at
         from ai_paper_review_metadata_confirmations c
         left join ai_paper_review_metadata_confirmation_undos u on u.confirmation_id = c.id
         where c.batch_id = ?1 and u.id is null order by c.created_at, c.id
