-- Literal substring matching: ASCII case-insensitive, other Unicode characters preserved.
-- No payload JSON, tool arguments, or provider checkpoints are read at search time.
with candidates as (
    select c.*, instr(lower(c.title), lower(?7)) > 0 as title_match,
      (select m.search_key from agent_search_messages m
       where m.conversation_id = c.id and m.last_ordinal < c.durable_count
         and not exists(select 1 from agent_rolled_back_turns removed
             where removed.conversation_id = m.conversation_id and removed.turn_id = m.turn_id)
         and instr(lower(m.body), lower(?7)) > 0
       order by m.start_ordinal, m.part_ordinal limit 1) as match_key
    from agent_conversations c
    where c.archived = ?1
      and (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=c.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=c.id)) = ?8
      and (?2 is null or c.created_at < ?2 or (c.created_at = ?2 and c.id < ?3))
      and (?5 is null or exists(select 1 from agent_conversation_bindings b
           where b.conversation_id = c.id and b.resource_kind = ?5 and b.resource_id = ?6))
)
select c.id, c.title, c.archived, c.durable_count, c.created_at, c.updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=c.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=c.id)),
       case when c.title_match then c.title
            else substr(m.body, max(1, instr(lower(m.body), lower(?7)) - 48), 320) end,
       case when c.title_match then null else m.item_id end,
       case when c.title_match then null else m.turn_id end,
       case when c.title_match then null else m.start_ordinal end,
       case when c.title_match then 1 else m.completed end
from candidates c left join agent_search_messages m on m.conversation_id = c.id and m.search_key = c.match_key
where c.title_match or c.match_key is not null
order by c.created_at desc, c.id desc limit ?4;
