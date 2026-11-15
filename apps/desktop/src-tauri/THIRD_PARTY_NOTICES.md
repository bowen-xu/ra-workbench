# Third-party notices

The application bundles the official OpenAI Codex executable from
`@openai/codex@0.146.0` (commit `e363b08c9175ac1cbe5893615dd2cb9ddf95043b`).
Codex is licensed under the Apache License 2.0. The complete license text is
available at <https://github.com/openai/codex/blob/rust-v0.146.0/LICENSE>.

The application-owned `research-agent-core` also contains modified Codex
thread-store lifecycle, completed-turn fork boundary semantics and response-assembly code from that commit. The native
`agent/model/responses_request.rs` and `responses_decoder.rs` adapt its Responses
request/event contracts and processing. Copyright 2025 OpenAI.
The full upstream LICENSE and NOTICE are bundled as `licenses/CODEX_LICENSE`
and `licenses/CODEX_NOTICE`. Modified files identify their origin; the source
migration ledger is `docs/architecture/shared-agent-codex-migration-map.md`.

The native `ai/codex_model_catalog/` adapts Codex models-manager cache,
discovery and context-window policy from the same commit. Its `bundled.json`
contains a sanitized metadata subset of `codex-rs/models-manager/models.json`.
Copyright 2025 OpenAI; Apache-2.0. The existing bundled Codex LICENSE and NOTICE
cover these modified policy/data adaptations.

## Ariakit React

Ariakit React 0.4.35 is used for accessible menus and dialogs.
Source: https://github.com/ariakit/ariakit

MIT License

Copyright (c) 2025-present Ariakit FZ-LLC

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Overleaf Code Check

The LaTeX Code Check worker is adapted from Overleaf Community Edition at
commit `28ad3b03b71cb4311decdcb55c36b33ec10d72db`.

Copyright Overleaf and its contributors.

Licensed under the GNU Affero General Public License, version 3. The complete
license is bundled as `licenses/OVERLEAF_CODE_CHECK_COPYING`. The pinned source
and local adapter are available in the Research Assistant source tree under
`apps/web/src/features/writing/editor/code-check/`.

Upstream source: https://github.com/overleaf/overleaf/blob/28ad3b03b71cb4311decdcb55c36b33ec10d72db/services/web/frontend/js/features/source-editor/languages/latex/linter/latex-linter.worker.ts

## CodeMirror merge

@codemirror/merge 6.12.2 supplies read-only Agent change review.
Source: https://code.haverbeke.berlin/codemirror/merge

MIT License

Copyright (C) 2018-2022 by Marijn Haverbeke <marijn@haverbeke.berlin> and others

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.

## Native Rust Docling

The native PDF daemon uses docling-pdf 2.45.7, source commit
`a90192cd3096b5db26db928ded4d3f024d7e4ed4` (MIT, copyright 2026 Artem Kustikov).
Full license: `licenses/docling/DOCLING_RS_LICENSE`. ONNX Runtime is a native MIT
licensed inference dependency. Model weights are delivered separately with their
license, model cards and source/checksum records; they are not part of the app installer.

The daemon includes an application patch to the pinned docling-pdf source under
`apps/desktop/docling-daemon/vendor/docling-pdf/`: retain untranscribed and
text-empty formula regions with their original text and location, and add
cooperative buffered-conversion cancellation at page boundaries. Source and
patch notes are shipped in the repository, with the upstream MIT notice intact.

The pinned `docling-core 2.45.7` is also patched under `vendor/docling-core/`
to retain multiple provenance entries for merged PDF text. The PDF assembler
preserves every source fragment's page, box and character range through column
and page merges. Both crates share the retained upstream MIT license above;
their repository patch notes list the modified source files.

The macOS Docling process uses tao 0.35.3 (Apache-2.0), tray-icon 0.24.2
(MIT OR Apache-2.0) and rfd 0.16.0 (MIT) for its own native status menu and
confirmation dialog. No third-party source is copied. Their complete licenses
are bundled under `licenses/docling/TAO_LICENSE`, `TRAY_ICON_LICENSE_MIT`,
`TRAY_ICON_LICENSE_APACHE` and `RFD_LICENSE`. Sources:
https://github.com/tauri-apps/tao, https://github.com/tauri-apps/tray-icon and
https://github.com/PolyMeilex/rfd.

## daff

daff 1.4.2 supplies table alignment in the optional note-table review adapter.
Source: https://github.com/paulfitz/daff/tree/6dbb7510b097369ac33b9126a88881f067f07a20

MIT License

Copyright (c) 2013 Paul Fitzpatrick &lt;paul@robotrebuilt.com&gt;

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
'Software'), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED 'AS IS', WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## VS Code completion list adaptation

The shared formula/writing completion list adapts the transactional row cache
and viewport rendering boundary from Microsoft VS Code commit
`e1a22a1ad4034ef7d2af09c5e0777d619cbb500b`,
`src/vs/base/browser/ui/list/rowCache.ts` and `listView.ts`. Local changes use
CodeMirror measurements, native scrolling, a single completion-row template and
a retained ARIA active row. The version-pinned pnpm patch also preserves the
existing CodeMirror MIT license.

MIT License

Copyright (c) 2015 - present Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Lucide completion SVG icons

The shared completion list copies the braces, sigma and file-text SVG paths
from lucide-react 1.28.0. They replace font/emoji icons to avoid font-dependent
layout in input transactions; no Lucide runtime or font dependency is added.

ISC License

Copyright (c) 2026 Lucide Icons and Contributors

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

---

The following Lucide icons are derived from the Feather project:

airplay, alert-circle, alert-octagon, alert-triangle, aperture, arrow-down-circle, arrow-down-left, arrow-down-right, arrow-down, arrow-left-circle, arrow-left, arrow-right-circle, arrow-right, arrow-up-circle, arrow-up-left, arrow-up-right, arrow-up, at-sign, calendar, cast, check, chevron-down, chevron-left, chevron-right, chevron-up, chevrons-down, chevrons-left, chevrons-right, chevrons-up, circle, clipboard, clock, code, columns, command, compass, corner-down-left, corner-down-right, corner-left-down, corner-left-up, corner-right-down, corner-right-up, corner-up-left, corner-up-right, crosshair, database, divide-circle, divide-square, dollar-sign, download, external-link, feather, frown, hash, headphones, help-circle, info, italic, key, layout, life-buoy, link-2, link, loader, lock, log-in, log-out, maximize, meh, minimize, minimize-2, minus-circle, minus-square, minus, monitor, moon, more-horizontal, more-vertical, move, music, navigation-2, navigation, octagon, pause-circle, percent, plus-circle, plus-square, plus, power, radio, rss, search, server, share, shopping-bag, sidebar, smartphone, smile, square, table-2, tablet, target, terminal, trash-2, trash, triangle, tv, type, upload, x-circle, x-octagon, x-square, x, zoom-in, zoom-out

The MIT License (MIT) (for the icons listed above)

Copyright (c) 2013-present Cole Bemis

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
