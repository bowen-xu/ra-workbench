-- Ordering-key maintenance changes only derived columns. It must not enter the
-- metadata-text projection, including during an outer catalog INSERT's triggers.
-- Source quote/link triggers from 0197/0199 still refresh their own metadata text.
drop trigger note_catalog_metadata_projection_update;
create trigger note_catalog_metadata_projection_update
 after update of title,source_label,source_id on note_page_catalog
 when old.title is not new.title or old.source_label is not new.source_label or old.source_id is not new.source_id
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id=new.id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;
