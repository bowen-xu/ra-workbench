//! Original synthetic export records, reused without changing their data.
use rusqlite::{Connection, params};
pub(super) fn seed_traceable_claim(connection: &Connection) {
    connection
        .execute(
            "insert into research_projects (id, title, summary, created_at)
             values ('project-1', 'Export project', '', '2026-08-04T00:00:00Z')",
            [],
        )
        .expect("insert project");
    connection
        .execute(
            "insert into bibliographic_items
             (id, item_type, title, created_at, updated_at)
             values ('paper-1', 'journalArticle', 'Export paper',
                     '2026-08-04T00:00:00Z', '2026-08-04T00:00:00Z')",
            [],
        )
        .expect("insert paper");
    connection
        .execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, 'private/object.pdf', 'application/pdf', 8, '2026-08-04T00:00:00Z')",
            ["a".repeat(64)],
        )
        .expect("insert content object");
    connection
        .execute(
            "insert into attachments
             (id, item_id, content_sha256, display_name, source_path, created_at)
             values ('attachment-1', 'paper-1', ?1, 'paper.pdf',
                     '/Users/example/private-paper.pdf', '2026-08-04T00:00:00Z')",
            ["a".repeat(64)],
        )
        .expect("insert attachment");
    connection
        .execute(
            "insert into source_anchors
             (id, item_id, attachment_id, content_sha256, page_index, quote_exact, created_at)
             values ('anchor-1', 'paper-1', 'attachment-1', ?1, 0,
                     'Synthetic evidence text', '2026-08-04T00:00:00Z')",
            ["a".repeat(64)],
        )
        .expect("insert source anchor");
    connection
        .execute(
            "insert into claims
             (id, project_id, statement, origin, provenance_kind, epistemic_role,
              revision, created_at, updated_at)
             values ('claim-1', 'project-1', 'A traceable claim', 'user',
                     'human_original', 'assertion', 1,
                     '2026-08-04T00:00:00Z', '2026-08-04T00:00:00Z')",
            [],
        )
        .expect("insert claim");
    connection
        .execute(
            "insert into evidence_links (id, claim_id, anchor_id, stance, rationale, created_at)
             values ('link-1', 'claim-1', 'anchor-1', 'supports', '',
                     '2026-08-04T00:00:00Z')",
            [],
        )
        .expect("insert evidence link");
    connection
        .execute(
            "insert into research_object_revisions
             (id, object_kind, object_id, revision, snapshot_json, editor_kind,
              change_reason, created_at)
             values ('revision-1', 'claim', 'claim-1', 1, ?1, 'human',
                     'manual_create', '2026-08-04T00:00:00Z')",
            params![r#"{"id":"claim-1","statement":"A traceable claim"}"#],
        )
        .expect("insert claim revision");
    connection
        .execute(
            "insert into note_pages (id, title, created_at, updated_at)
             values ('note-page-1', 'Export note page',
                     '2026-08-04T00:00:00Z', '2026-08-04T00:00:00Z'),
                    ('reading-page-1', 'Reading document',
                     '2026-08-04T00:00:00Z', '2026-08-04T00:00:00Z')",
            [],
        )
        .expect("insert note page");
    connection
        .execute(
            "insert into note_blocks
             (id, scope_kind, page_id, block_type, content_json, plain_text,
              created_at, updated_at)
             values ('page-block-1', 'page', 'note-page-1', 'paragraph',
                     '{\"kind\":\"inline\",\"items\":[{\"type\":\"text\",\"text\":\"Page text\",\"styles\":[]}]}', 'Page text',
                     '2026-08-04T00:00:00Z', '2026-08-04T00:00:00Z'),
                    ('reading-body-1', 'page', 'reading-page-1', 'paragraph',
                     '{\"kind\":\"inline\",\"items\":[{\"type\":\"text\",\"text\":\"Reading note text\",\"styles\":[]}]}',
                     'Reading note text', '2026-08-04T00:00:00Z',
                     '2026-08-04T00:00:00Z')",
            [],
        )
        .expect("insert note blocks");
    connection
        .execute(
            "insert into reading_note_sources
             (block_id, page_id, note_type, color, current_anchor_id,
              paper_id_snapshot, paper_title_snapshot, attachment_id_snapshot,
              attachment_name_snapshot, content_sha256, page_index, rects_json,
              quote_exact, quote_prefix, quote_suffix, locator_version, created_at, updated_at)
             values ('reading-note-1', 'reading-page-1', 'annotation', '#f4cf65',
                     'anchor-1', 'paper-1', 'Export paper', 'attachment-1', 'paper.pdf', ?1, 0, '[]',
                     'Synthetic evidence text', '', '', 1, '2026-08-04T00:00:00Z',
                     '2026-08-04T00:00:00Z')",
            ["a".repeat(64)],
        )
        .expect("insert canonical reading source");
}
