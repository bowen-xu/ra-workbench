# Research Assistant patch

Source: published `docling-core 2.45.7`, commit
`a90192cd3096b5db26db928ded4d3f024d7e4ed4`, `crates/docling-core`.
The registry package's `.cargo_vcs_info.json` records this provenance.
License: MIT, copyright 2026 Artem Kustikov; see LICENSE. Cargo cache markers
and the standalone lockfile are omitted; original package files are retained.

`src/document.rs` adds `Node::TextProvenance`, carrying the existing `TreeProv`
type for every fragment of a merged paragraph. The PDF assembler preserves each
fragment rather than retaining only its first box. `src/json.rs` writes every
fragment into the text item's `prov` array, for both Value and streaming exports,
using existing rounding and page clamping. A single fragment still uses the
existing compact typed text item. No wire-schema extension is necessary.

`src/lib.rs` exports the existing text-unescaping helper so merge character ranges
count exactly the same Unicode text as JSON export. `src/document.rs`,
`src/markdown.rs`, `src/doclang.rs`, `src/latex.rs`, `src/chunker.rs` and
`src/redact.rs` traverse the wrapper transparently, preserving ordinary text
rendering, chunking and mutation behavior. Other upstream sources are unchanged.

Application regressions live in `../../tests/text_provenance.rs`; they exercise
actual PDF assembly, merges, Unicode/escaped character spans, dehyphenation,
rotation, Value/streaming JSON export and single-render Markdown without models.
The daemon lockfile pins this path patch alongside the PDF crate patch. Remove
the patches only after these regressions pass against an upstream replacement.
