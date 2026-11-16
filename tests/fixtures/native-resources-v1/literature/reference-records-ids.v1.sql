select id from reference_records where citing_paper_id = ?1
         order by updated_at desc, id