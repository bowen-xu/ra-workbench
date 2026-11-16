insert into paper_generation_conversations(paper_id,conversation_id) values(?1,?2)
        on conflict(paper_id) do update set conversation_id=excluded.conversation_id