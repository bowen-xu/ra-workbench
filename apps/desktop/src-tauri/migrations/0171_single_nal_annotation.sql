-- Keep one link per target. Extra pages, blocks and revision history remain in
-- the Notes library; no note content is deleted or merged during this upgrade.
delete from nal_annotations where id not in (
    select min(id) from nal_annotations group by concept_id, statement_id
);
create unique index nal_single_concept_note on nal_annotations(concept_id) where concept_id is not null;
create unique index nal_single_statement_note on nal_annotations(statement_id) where statement_id is not null;
update nal_state set revision = revision + 1 where id = 1;
