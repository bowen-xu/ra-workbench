select exists(select 1 from ai_paper_review_metadata_confirmations
         where batch_id = ?1 and field_name = ?2)
