create table reading_note_sources_next (
    block_id text primary key references note_blocks(id) on delete cascade,
    current_anchor_id text references source_anchors(id) on delete set null,
    paper_id_snapshot text not null,
    paper_title_snapshot text not null default '',
    attachment_id_snapshot text not null,
    attachment_name_snapshot text not null default '',
    content_sha256 text not null,
    page_index integer not null check(page_index >= 0),
    page_label text,
    rects_json text not null default '[]' check(json_valid(rects_json)),
    quote_exact text not null default '',
    quote_prefix text not null default '',
    quote_suffix text not null default '',
    locator_version integer not null default 1 check(locator_version >= 1),
    created_at text not null,
    updated_at text not null
);

insert into reading_note_sources_next (
    block_id, current_anchor_id, paper_id_snapshot, paper_title_snapshot,
    attachment_id_snapshot, attachment_name_snapshot, content_sha256,
    page_index, page_label, rects_json, quote_exact, quote_prefix, quote_suffix,
    locator_version, created_at, updated_at
)
select block_id, current_anchor_id, paper_id_snapshot, paper_title_snapshot,
       attachment_id_snapshot, attachment_name_snapshot, content_sha256,
       page_index, page_label, rects_json, quote_exact, quote_prefix, quote_suffix,
       locator_version, created_at, updated_at
from reading_note_sources;

drop table reading_note_sources;
alter table reading_note_sources_next rename to reading_note_sources;
create index reading_note_sources_paper_page_idx
    on reading_note_sources(paper_id_snapshot, page_index);
create index reading_note_sources_current_anchor_idx
    on reading_note_sources(current_anchor_id);
