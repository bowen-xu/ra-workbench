---
name: edit-structured-resources
description: Read and edit managed paper portraits, structured reading, paper-specific templates and global reading templates as JSON at authorized source:// addresses. Use when creating, repairing or revising these objects.
---

Discover the authorized resource and read its current candidate with `research_read_file`. The same address exists before the first generated content. Do not invent an ID, append a filename or use an operating-system path.

Read only the relevant format reference: [portrait](references/portrait.md), [structured reading](references/reading.md), or [reading templates](references/templates.md). These instructions do not grant access; the current conversation's resource permissions always apply.

Use `research_apply_patch` with exact, unique context. An empty existing object is an Update File target. A validation error saves a durable repair draft and returns diagnostics: read candidate again and patch that same draft. Do not repeat the entire task or discard an invalid response. Re-read on a version conflict. Recover the original tool request after an uncertain result instead of replaying writes.

The host pre-creates blank portrait and structured-reading objects before the task starts. Always use `*** Update File: source://...` for these targets, including their first write; never `Add File`. Read the current candidate first. For truly empty text, use an `@@` hunk followed by `+` lines containing the complete JSON. For nonempty text, match exact existing lines with `-`/`+` replacements. A no-match error requires a new read, not a guessed patch. After an `applied` receipt, stop editing that target unless a separate requested template proposal remains.

For new citations in an ordinary chat, read the authorized PDF with `research_read_source` and `view: "structured"`. Follow its ordinary character pagination. This view supplies frozen `evidenceItemId` values and literal `quoteExact` text; the default raw view remains available for normal Docling inspection. An address or an ID mentioned in earlier messages does not authorize reading additional attachments.

`applied` means the formal content was durably saved; pending Diff is separate and remains for the user to Keep or Undo. Never claim manual evidence verification, fabricate review markers, or issue acceptance commands. Completion must be based on the target's durable application receipt, not on producing JSON in a final chat response.

Template writes follow the same Diff lifecycle as portraits and reading: valid changes save immediately with the review baseline retained. Users review with Keep/Undo. Never fabricate review acceptance or verification. Invalid and conflicting drafts remain available for repair. Historical approvalRequired receipts are legacy recovery data, not a new publication workflow.

## Registered resource paths
Paper resources use `source://library/<paper-id>/<resource-id>`. Global reading templates are this skill's reusable assets: `source://agents/skills/edit-structured-resources/assets/reading/<template-id>.json`. Use `research_read_file` to read the exact registered JSON object. The directory address without a filename is the authorized creation target for `Add File`; the host allocates the new ID and returns the JSON asset address. Use `references/templates.md` for its schema; never guess a disk path.
