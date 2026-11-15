/** Shared document review presentation and CodeMirror adapters. Hosts retain
 * document identity, HBS state, persistence and the editor's native history. */
export { EditorDiffControls } from "../components/editor-diff/EditorDiffControls";
export { EditorDiffHighlight, DiffHighlight } from "../components/editor-diff/EditorDiffHighlight";
export type { EditorDiffHighlightAppearance } from "../components/editor-diff/EditorDiffHighlight";
export { chunkActionDecorations } from "../components/editor-diff/codeMirrorChunkActions";
export { resolveTextChunk, resolveTextDiff, textChanges, textDiffResolution } from "../components/editor-diff/textDiffCommands";
export { textReviewBaseline } from "../components/editor-diff/textReviewBaseline";
