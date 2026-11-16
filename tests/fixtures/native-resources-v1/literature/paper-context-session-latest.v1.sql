select id from paper_ai_reading_sessions
             where paper_id = ?1 and status = 'active'
             order by updated_at desc limit 1