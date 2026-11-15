import type {NoteCatalogRuntime} from "@research-assistant/data/notes";
import {NotesGatewayError,NOTE_CATALOG_COLLATION_VERSION} from "@research-assistant/data/notes";

// Generated from the same Rust crate used by native projection functions.
// Loading is lazy: a persisted recent-page query does not initialize Wasm.
let pending: Promise<NoteCatalogRuntime> | undefined;
interface QueryWasmModule {
  default(input: {module_or_path: URL}): Promise<unknown>;
  note_collation_version(): string;
  note_normalization_version(): string;
  note_sort_key(text: string): Uint8Array;
  note_reverse_key(key: Uint8Array): Uint8Array;
  note_fold(text: string): string;
  note_query_terms(text: string): readonly string[];
  note_display_title(body: string, quote: string, fallback: string): string;
}
function fromWasm<T>(code: "invalid_input" | "index_failed", operation: () => T): T {
  try { return operation(); }
  catch (reason) { throw reason instanceof NotesGatewayError ? reason : new NotesGatewayError(code,reason instanceof Error ? reason.message : String(reason)); }
}
export function loadBrowserNoteCatalogRuntime(): Promise<NoteCatalogRuntime> {
  if (pending) return pending;
  pending = (async () => {
    const entry = new URL("../../../../.generated/note-query/research_note_query_wasm.js", import.meta.url);
    const imported: Record<string,unknown> = await import(/* @vite-ignore */ entry.href);
    if (["default","note_collation_version","note_normalization_version","note_sort_key","note_reverse_key","note_fold","note_query_terms","note_display_title"].some(name => typeof imported[name] !== "function"))
      throw new NotesGatewayError("index_failed", "note_catalog_runtime_exports：笔记查询组件接口不匹配，请重新构建应用。");
    const module = imported as unknown as QueryWasmModule;
    const wasm = new URL("../../../../.generated/note-query/research_note_query_wasm_bg.wasm", import.meta.url);
    await module.default({module_or_path: wasm});
    const runtime: NoteCatalogRuntime = {
      collationVersion: module.note_collation_version(), normalizationVersion: module.note_normalization_version(),
      sortKey: text => fromWasm("index_failed",() => module.note_sort_key(text)), reverseKey: module.note_reverse_key, fold: module.note_fold,
      terms: text => fromWasm("invalid_input",() => module.note_query_terms(text)), displayTitle: module.note_display_title,
    };
    if (runtime.collationVersion !== NOTE_CATALOG_COLLATION_VERSION || !runtime.normalizationVersion.startsWith("rust-unicode-"))
      throw new NotesGatewayError("index_failed", "note_catalog_runtime_version：笔记查询组件版本不匹配，请重新构建应用。");
    return runtime;
  })().catch(reason => {
    pending = undefined;
    throw reason instanceof NotesGatewayError ? reason : new NotesGatewayError("index_failed", `note_catalog_runtime_unavailable：${reason instanceof Error ? reason.message : String(reason)}`);
  });
  return pending;
}
