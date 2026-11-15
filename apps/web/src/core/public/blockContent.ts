/** Shared block-document composition. Consumers own schemas, managed assets,
 * document sessions and resource resolution; importing this creates no note. */
export { baseBlockEditorSchema, blockEditorDefaults } from "../components/block-editor/index";
export { withCaptionBlocks, withoutCaptionBlocks } from "../components/block-editor/captionBlockProjection";
export { ManagedAssetIngress } from "../components/block-editor/ManagedAssetIngress";
export type { AssetInsertion } from "../components/block-editor/ManagedAssetIngress";
export { NoteEditorSurface } from "../components/block-editor/NoteEditorSurface";
export { ImageBlockPresentation } from "../components/block-editor/ImageBlockPresentation";
export type { ImageAlignment } from "../components/block-editor/ImageBlockPresentation";
export type { ImageResizeSource } from "../components/block-editor/ImageBlockResize";
export type { BlockSourceNavigation } from "../components/block-editor/BlockSourceNavigation";
export type { MediaBlockEditor } from "../components/block-editor/mediaBlockFocus";
