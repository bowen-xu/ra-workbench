select id,
                        case when length(statement) > 240
                             then substr(statement, 1, 239) || '…' else statement end,
                        scope_note
                 from definition_claims where id = ?1
