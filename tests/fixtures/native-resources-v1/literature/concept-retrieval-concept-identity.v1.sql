select c.revision,
                    coalesce(nullif(c.label, ''), preferred.text, '未命名概念'),
                    coalesce(preferred.text, nullif(c.label, ''), '未命名概念')
             from concepts c
             left join concept_terms link
               on link.concept_id = c.id and link.relation_kind = 'preferred'
             left join terms preferred on preferred.id = link.term_id
             where c.id = ?1 and c.deleted_at is null