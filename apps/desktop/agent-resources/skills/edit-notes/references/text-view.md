<!-- Generated from notes/text_view/protocol.json; edit that source. -->
Note text view v1. This is an authoring view of the original named note at its existing source:// address, not a Markdown file. Do not add .md or replace names with IDs.
Use CommonMark headings, paragraphs, lists, task lists, fenced code, quotes, dividers and LaTeX formulas: \(...\) for inline math and \[...\] for display math (put display delimiters on separate lines in their own paragraph, with blank lines before and after). Use these delimiters when writing notes; do not wrap formulas in code or use $ or $$ as the authoring convention. Dollar-based editor shortcuts are separate UI actions. Ordinary [label](URL) links are distinct from object references. Internal links use source:// addresses, including in note content. Reference kind is a separate XML attribute; use the corresponding library/notes/writings/agents/assets namespace. Internal resource URIs exclusively use source://; no alternate protocol is supported.
XML extensions are strict and case-sensitive. Unknown tags/attributes and arbitrary HTML are rejected. XML uses standard entity escaping; use CDATA when text contains raw Markdown with < or &. Code and math treat XML literally.
Inline: <reference kind="paper|note_page|reading_note|claim|manuscript" target="source://notes/note-id">label Markdown</reference>, <u>inline Markdown</u>, <span color="red" background="yellow">inline Markdown</span>. <strong>, <em>, and <s> wrap inline Markdown with explicit bold, italic, or strike styling when delimiter boundaries would be ambiguous. Preserve numeric character references such as &#32; at text boundaries; they represent literal whitespace, not formatting. Colors: default,gray,brown,orange,yellow,green,blue,purple,pink,red.
Blocks: <image src="asset://handle" name="name" mime="image/png" caption="caption"/>, <file src="asset://handle" name="name" mime="type" caption="caption"/>.
Import an attachment or captured image only from a discovered authorized attachmentId or captureId using src="import://handle". The backend copies its verified bytes into managed staging and returns an asset handle in subsequent reads. A normal hyperlink never imports bytes.
<reference kind="..." target="source://notes/note-id" display="block|reading-note" label="name"><![CDATA[quoted text]]></reference> preserves object-reference snapshots.
<child-page target="source://notes/note-id" label="name"/> represents a child page, not an ordinary link. Removing it proposes recycling that child's subtree. Add File uses an authorized parent note source:// address and title frontmatter to create a child; the backend returns its allocated address. Move to uses source://notes/parent-id as the new parent, or source://notes/ to move to root, preserving the moved page ID. Normal reads and returned addresses always use source://notes/page-id.
<table><tr><td><![CDATA[inline Markdown]]></td></tr></table> preserves ragged or empty tables. GFM tables are allowed for rectangular tables; separator lines are syntax, not extra note rows.
<toggle><![CDATA[inline Markdown title]]></toggle> represents a collapsible list item. Put its child blocks in <children><![CDATA[block Markdown]]></children> immediately afterward; children remain readable/editable while the UI is collapsed. A heading followed by <block-format toggle="true"/> is a toggle heading; toggle="false" or removing that attribute makes it a regular heading. Open/closed is local reading state, not document content. Do not use > for toggles: CommonMark > remains a quote.
<children><![CDATA[block Markdown]]></children> immediately follows its parent block. <block-format align="left|center|right|justify" color="..." background="..."/> immediately follows the block it formats.
Unknown future blocks are represented by an opaque <file src="opaque://handle" name="..."/>; preserve or move it unchanged, or remove it explicitly. Never invent opaque or asset handles.
The frontmatter is exactly --- / title: a YAML string / ---. Do not add document IDs, block IDs, versions or internal JSON.
Patch expected lines must match exactly and uniquely. Add distinguishing context on ambiguity. Read before updating/deleting. New note changes are saved together with a separate review baseline; pending Diff remains until user Keep/Undo. Saving does not mean Keep. Historical results marked proposed are not saved; recover their original request instead of replaying. Reads default to current content; view=saved selects acknowledged saved content.

XML attribute whitelist:
- reference: kind, target, display, label
- image: src, name, mime, caption
- file: src, name, mime, caption
- child-page: target, label
- children: none
- table: none
- tr: none
- td: none
- u: none
- span: color, background
- block-format: align, color, background, toggle
- strong: none
- em: none
- s: none
- toggle: none

Examples:

Replace one uniquely matched paragraph

```text
*** Begin Patch
*** Update File: source://notes/00000000-0000-0000-0000-000000000001
@@
-Original paragraph.
+Revised paragraph.
*** End Patch
```

Object references differ from links

```text
---
title: "Research notes"
---

[Website](https://example.com)

<reference kind="paper" target="source://library/paper-id">**Paper**</reference>

```

Import an authorized attachment

```text
---
title: "Attachment"
---

<file src="import://authorized-attachment-id" name="paper.pdf" mime="application/pdf" caption="Source"/>

```

Inline and display formulas

```text
---
title: "Formulas"
---

Energy is \(E=mc^2\).

\[
\sum_{i=1}^{n} x_i \ge 5
\]

```

Collapsible list with editable children

```text
---
title: "Plan"
---

<toggle><![CDATA[**Details**]]></toggle>

<children><![CDATA[Child paragraph.

- Nested list]]></children>

```
