select id, case when length(statement) > 240
                         then substr(statement, 1, 239) || '…' else statement end,
                scope_note from definition_claims
         where ?1 = '' or instr(lower(statement || ' ' || scope_note), lower(?1)) > 0
         order by updated_at desc limit 15
