# Structured reading JSON

Use `kind: "paper_overview_unit_v1"` and `schemaVersion: 1`. Preserve the task's exact profile and template snapshot, ordered field IDs and definitions. For global-template runs, the snapshot identifies `templateResourceId` and `templateResourceRevision`; never place a template ID in `portraitCandidateId`. Historical portrait snapshots remain readable.

Each `fields` entry has `field`, `status` (`filled`, `partial` or `not_found`) and `statements`. A statement has stable `id`, `text`, `epistemicRole` (`source_paraphrase`, `synthesis`, `inference` or `uncertain`), `evidenceItemIds` and `evidenceQuotes` entries `{evidenceItemId,quote}`. Each cited source in a template-based statement needs one nonempty literal continuous quote copied from that exact frozen source. Prefer the shortest sufficient quote without paraphrasing or truncating it. `not_found` has no statements.

Read the current object before patching it. Retain unmodified fields, statement IDs and human text. Changing reading content does not authorize switching templates. If the framework is unsuitable, propose the separate template change for user confirmation.

Canonical resources may also contain `retainedFields`, which preserves bodies removed from the active template during a confirmed template change. It uses the same field shape and original stable IDs. You may edit existing retained fields, but cannot create new retained IDs to bypass template confirmation. Omitting this property during regeneration does not delete saved retained bodies. When a template reintroduces a field with the same ID, its saved body is restored to the active fields.
