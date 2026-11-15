/** Shared workbench capability; implementation and lifecycle remain owned by Core. */
export { getAppStorage } from "../runtime/appStorage";
export { flushStorage, storagePersistence } from "../runtime/storagePersistence";
export { decodeMockState, encodeMockState } from "../runtime/mockStateCodec";
