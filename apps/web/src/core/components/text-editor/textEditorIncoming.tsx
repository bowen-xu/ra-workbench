/** Compatibility entry for existing editor hosts. Conflict presentation belongs
 * to the shared review component, not to the HBS experiment. */
export * from "../editor-diff/codeMirrorIncoming";
export { EditorReviewActionGroup as IncomingChunkActions } from "../editor-diff/EditorReviewActions";
