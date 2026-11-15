/** Compile-time checks over the real public module; no alternate renderer,
 * query owner, or runtime registration is created by these types. */
type Compatible<Expected, Actual extends Expected> = Actual;

export type SdkNotePresentationContracts = {
  readonly presentation: Compatible<typeof import("@research-assistant/workbench-sdk/note-presentation"), typeof import("../core/public/noteDocuments")>;
  readonly metadata: Compatible<typeof import("@research-assistant/workbench-sdk/note-metadata"), typeof import("../core/public/noteDocuments")>;
};
