# Research Assistant patch

Source: published `docling-pdf 2.45.7`, upstream commit
`a90192cd3096b5db26db928ded4d3f024d7e4ed4`, `crates/docling-pdf`.
The registry package's `.cargo_vcs_info.json` records this provenance.
License: MIT, copyright 2026 Artem Kustikov; see LICENSE.

The first modified upstream source file is `src/assemble.rs`: a formula with
no CodeFormula enrichment remains `Node::Formula` with empty LaTeX, its original
extracted text and its location. Upstream replaces it with an unlocated paragraph
containing an HTML comment. That loses the JSON formula label and provenance,
preventing overlays and source tools from locating a correctly detected region.
An empty extracted string must not discard a detected formula before that branch.
The enriched formula branch, inference models and confidence thresholds stay intact.

`../../tests/formula_provenance.rs` exercises the actual assembler and JSON
serializer without model downloads or a PDF inference run. The consuming Cargo
manifest pins this source with a path patch, so builds do not modify Cargo's cache
or depend on the ignored reference checkout. Original package files are retained;
Cargo cache markers and the package's standalone lockfile are omitted.

On an upstream upgrade, compare this branch and remove the patch only when the
same regression passes against the replacement. Parser execution identity is
versioned independently of model resources to invalidate old service/cache reuse.

A second patch in src/lib.rs adds set_cancel_flag for buffered conversion. It
checks an optional atomic flag before decode, at renderer/worker page boundaries
and before returning an assembled result. It reuses the existing bounded page
queues and progress hook; already-running page/model work finishes. The daemon
clears both hooks before reusing its warm Pipeline. Streaming conversion, model
weights, preprocessing and thresholds are unchanged. tests/cancellation.rs and
tests/task_progress.rs cover pre-decode cancellation, actual FP32 progress and
post-cancel daemon reuse. Service discovery is fenced by SERVICE_REVISION without
invalidating previously successful parse caches or requiring model downloads.

Parser revision 2 also patches paragraph assembly in `src/assemble.rs` and adds
`src/text_provenance.rs` (registered by `src/lib.rs`). Normal paragraphs carry
the pinned core patch's `TextProvenance` wrapper. Same-page column merges retain
each source box; cross-page continuation merges concatenate the fragment list
and shift character ranges over unescaped Unicode text, including removal of a
wrap hyphen. Original point coordinates are rotated to display space and every
fragment receives its actual 1-based page number before document-level merges.
The existing merge heuristics, models and confidence thresholds are unchanged.
`../../tests/text_provenance.rs` covers these assembly/export contracts.
