---
name: edit-notes
description: Read and revise Research Assistant notes through Markdown/XML authoring views at existing source:// addresses using research_read_file and research_apply_patch. Use for note edits and note page or attachment changes through chat.
---

Read [the versioned text-view protocol](references/text-view.md) before editing special references, attachments, styles, nesting, or page structure. This resource is generated from the same definition used by native XML validation and tool instructions.

Discover authorized sourceUrl addresses with research_find_sources or research_list_sources. Read the current candidate with research_read_file; use view=saved only when comparing with the saved page. Use the returned existing source address, with the original note title or file name; never add .md to a note or create a new mount namespace. Add File addresses an authorized parent note and allocates a child identity; follow the returned source address after creation or movement.

Submit a Codex-format text patch to research_apply_patch. Context is exact and must be unique. On ambiguity or version conflict, re-read and add distinguishing context. The model never supplies block IDs, document JSON, database arguments, or acceptance commands.

Check each target's result. applied updates an authorized execution file copy, or saves a note together with its separate review baseline. Manuscript delivery remains separate. Pending note Diff is preserved after saving; only user Keep advances its baseline. Historical proposed results retain their deferred-save recovery contract. Mixed batches can partially succeed. Recover an unknown original request before any new write; do not resend successful targets.
