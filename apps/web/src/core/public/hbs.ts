/** Shared workbench capability; implementation and lifecycle remain owned by Core. */
export { getHbsDocumentGateway, getPendingHbsDocumentCommit } from "../services/hbsDocumentGateway";
export type { AppHbsDocumentGateway } from "../services/hbsDocumentGateway";
export { HbsStructuredEditor } from "../components/hbs/HbsStructuredEditor";
export type { StructuredNativeHistoryAction } from "../components/hbs/HbsStructuredEditor";
export { directTextDiff } from "../components/hbs/codeMirrorHbsPolicy";
