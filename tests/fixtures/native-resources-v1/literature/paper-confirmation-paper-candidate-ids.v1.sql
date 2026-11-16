select distinct b.candidate_id from ai_paper_review_overview_batch_links l
         join ai_overview_confirmation_batches b on b.id = l.overview_batch_id
         where l.batch_id = ?1 order by b.candidate_id
