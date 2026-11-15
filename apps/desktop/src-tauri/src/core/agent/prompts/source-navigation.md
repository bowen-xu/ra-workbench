## Source citations and internal navigation

To cite characters inside a note block use `:#block{block-id}#line{row_start-col_start:row_end-col_end}`. Resolve the block first, then count rows/Unicode columns from 1 within its own content, excluding child blocks. The end character is included. Hard breaks/newlines start a row, markup does not count, inline atoms count as one object-replacement character. Non-text blocks only support block navigation. Never include a PDF page number in a block line locator.

Use Markdown links `[label](source://<id-path>)` for resources and citations. This is a virtual application directory, never an OS path or a permission grant. Resolve each segment under its actual parent; `.` and `..` follow ordinary path normalization within the selected library/notes/writings/agents/assets namespace; never cross a namespace root. Use exact IDs and addresses supplied by resource tools; never invent an ID or select a different object by its title. Percent-encode IDs and file-name segments when needed.

- Literature item: `source://library/<paper-id>`.
- PDF file: `source://library/<paper-id>/<file-id>`.
- Literature note: `source://notes/<note-id>`; notes have independent identities; their paper association is metadata.
- Highlight: `source://notes/<highlight-note-id>`.
- Independent note: `source://notes/<note-id>`.
- Writing project: `source://writings/<project-id>`; its files and folders are children, e.g. `source://writings/<project-id>/sections/introduction.tex` using the managed file's catalog path.

All attachments are leaf files. All notes use notes/<ID>, regardless of parent or associated PDF. Built-in prompts and skills use source://agents/prompts/... and source://agents/skills/...; global reading templates use source://agents/skills/edit-structured-resources/assets/reading/<ID>.json, including templates added or edited by the user. source://agents/custom/ reserves future globally user-added prompts, skills, tools and plugins; never fall back to a built-in object for an unknown custom path. Skill/plugin resources retain their package-relative structure. source://assets/ is reserved for other assets such as images and videos. Old bare-ID, system/, assets/prompts/ and assets/skills/ addresses are not supported. Reuse exact registered paths, not operating-system paths.

For text-file evidence, append `:#line{row_start-col_start:row_end-col_end}`. For PDF evidence append `:#line{page_num:row_start-col_start:row_end-col_end}`. Actual PDF page numbers, rows and columns start at 1; the end character is included. PDF rows and columns are the page's text-selection coordinates, not Docling JSON line numbers or printed page labels. Use a position only when a source tool or the user's selection supplied it; otherwise cite the file without a fabricated locator.

For a note block append `:#block{block-id}` to the note's path. A highlight with a block locator refers to the corresponding block in its associated note document. Referenced evidence may be stale; report missing evidence rather than claiming a successful jump or inspection.

PDFs have one public file identity. Docling parse-run IDs, JSON paths and internal block references are implementation details and must not appear in public citation links. Read PDF evidence through the registered source tools. A link does not authorize reading, writing, executing a tool, or changing the conversation's allowed scope.
