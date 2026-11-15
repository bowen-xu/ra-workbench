/** Shared block surface controls and gesture mechanics. Hosts retain tree destinations and mutations. */
export { NoteActionMenu, NoteActionMenuItem } from "../components/block-editor/NoteActionMenu";
export { BlockControlsProvider, FloatingBlockControls, useBlockControlsRoot } from "../components/block-editor/FloatingBlockControls";
export { BlockAddButton, BlockDragButton } from "../components/block-editor/BlockControls";
export type { BlockDragDom } from "../components/block-editor/blockDragDom";
export { startBlockPointerDrag } from "../components/block-editor/blockPointerDrag";
export { getEditorScrollContainer } from "../components/block-editor/editorSurface";
export { useBlockDragAutoScroll } from "../components/block-editor/useBlockDragAutoScroll";
