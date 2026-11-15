-- Canonical document writes update blocks and then advance note_pages.revision
-- in the same transaction. Refresh the display title at that commit boundary,
-- not once per block: per-block tree traversal makes large saves quadratic.
-- Creation imports the body before adding reading_note_sources, whose trigger
-- builds the initial title. Metadata/source/favorite triggers remain unchanged.
drop trigger note_page_catalog_blocks_insert;
drop trigger note_page_catalog_blocks_update;
drop trigger note_page_catalog_blocks_delete;
