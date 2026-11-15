/** Type-only conformance. Shared runtime values remain in their Core modules. */
type Compatible<Expected, Actual extends Expected> = Actual;
export type SdkNoteSessionContracts = {
  readonly documents: Compatible<typeof import("@research-assistant/workbench-sdk/note-documents"), typeof import("../core/public/noteDocuments")>;
  readonly resources: Compatible<typeof import("@research-assistant/workbench-sdk/note-resources"), typeof import("../core/public/noteResources")>;
  readonly workbench: Compatible<typeof import("@research-assistant/workbench-sdk/workbench"), typeof import("../core/public/workbench")>;
};
