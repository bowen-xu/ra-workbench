/** Existing writing record/CAS access. Connection, schema, registration and
 * projection ownership stay private to Core and application assembly. */
export {writingRecords} from "../features/writing-storage/writingRecordAccess";
export type {WritingRecordAccess} from "../features/writing-storage/writingRecordAccess";
export {writingFileKey} from "../features/writing-storage/browserWritingRecords";
export type {WritingRecordChange, WritingRecordSelection, WritingRecordSnapshot} from "../features/writing-storage/browserWritingRecords";
export {decodeWritingBody, decodeWritingDraft, decodeWritingFile, decodeWritingProject, decodeWritingSession} from "../features/writing-storage/writingRecordCodec";
export type {StoredWritingProject} from "../features/writing-storage/writingRecordCodec";
