# Shared document review presentation

`EditorDiffChunkActions` owns local Undo/Keep and optional conflict choices;
`EditorDiffControls` owns navigation and bulk actions. `EditorDiffHighlight`
owns Diff paint for the same review surface.

```tsx
<EditorDiffHighlight appearance={{ addedUnderline: "#2468ce" }}>
  <ExistingEditor />
  <DiffHighlight kind="removed">old fragment</DiffHighlight>
  <DiffHighlight kind="added">new fragment</DiffHighlight>
</EditorDiffHighlight>
```

- Default appearance is the current CodeMirror 6.12.2 light unified editor:
  pale removed/added lines, red fill for removed characters, a 2px green
  underline for added characters with no solid green fill or red underline.
- `EditorDiffHighlight.css` contains the shared palette and native class maps.
  CodeMirror keeps its native decorations; the note adapter keeps real
  ProseMirror blocks, cells and complete atoms. React fragments use
  `DiffHighlight` (`text`, `block` or `atom` scope). No editor DOM is sliced or
  replaced to obtain the common style.
- Layout, padding, syntax coloring, selection and editor ownership remain in
  each renderer. Engine adapters map native classes here; feature and preview
  CSS must not introduce another Diff palette.
- `appearance` injects typed, instance-local CSS variables. Omitted values
  inherit an enclosing boundary; independent editors keep default paint.
  The boundary uses `display: contents`, has no scroll/focus owner, and does
  not recreate editor state when its appearance changes.
- Text/HBS review consumers pass `reviewConfiguration.highlight` (or
  `configuration.highlight`). Notes and LaTeX expose `diffHighlight`;
  `TextDiffEditor`, `WritingTextDiff`, and `StructuredTextDiff` expose
  `appearance`. All can instead inherit a shared outer boundary.
- Presentation does not compute changes, choose operation ranges, modify HBS
  state, persist data or create an undo stack. Diff and history adapters stay
  independently injectable.

The standalone HBS reference remains frozen. Its default text rendering is
the comparison control for this shared component, not another implementation
to overwrite when adding application renderers.

`EditorReviewLocationActions` is the shared control for paired position
fragments. The current location shows `old / Keep` (`jump to old`); the old
location shows `new / Undo` (`jump to new`). Hosts supply scoped scrolling and
focus through `onJump`, and route `onResolve` to the same original decision.
The control never computes a move, changes selection or creates history.
It uses the existing action palette and `ControlHint`, without another Diff
style. Content-only decisions retain their ordinary Undo/Keep controls.
