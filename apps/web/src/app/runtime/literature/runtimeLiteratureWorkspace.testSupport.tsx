/** Exact test-only assembly entry. Re-exports real production APIs; no UI or
 * replacement runtime. Production plugins cannot import this support entry. */
export {createAppRuntime} from "../createAppRuntime";
export {getAppRuntime, selectAppRuntime} from "../../../core/runtime/appRuntime";
export {NotesProvider, useNotesClient} from "../../../core/features/notes/NotesClient";
export {getNotesGateway} from "../../../core/features/notes/gateways";
export {createRuntimeLiteraturePort} from "./runtimeLiterature";
