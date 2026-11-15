import type {LoadedRuntimePlugin} from "../../../core/runtime/plugins/loader";
import {getAppRuntime} from "../../../core/runtime/appRuntime";
import {invoke as invokePlatform} from "../../../core/runtime/platformCore";
import {createTauriWritingGateway} from "./tauriWritingGateway";
import {createTauriWritingCheckpointGateway} from "./tauriWritingCheckpointGateway";
import {createTauriWritingNoteAssetGateway} from "./tauriWritingNoteAssetGateway";
import {nativeWritingOperationFailure} from "./nativeWritingPreview";

/** The existing adapters keep argument construction and response decoding. */
const projectCommands = {
  import_writing_note_asset: "import_runtime_writing_note_asset",
  read_writing_note_asset: "read_runtime_writing_note_asset",
  export_writing_note_asset: "export_runtime_writing_note_asset",
  import_manuscript_project: "import_runtime_manuscript_project",
  duplicate_manuscript: "duplicate_runtime_manuscript",
  export_manuscript_source: "export_runtime_manuscript_source",
  export_manuscript_pdf: "export_runtime_manuscript_pdf",
  import_manuscript_resources: "import_runtime_manuscript_resources",
  forward_manuscript_synctex: "forward_runtime_manuscript_synctex",
  inverse_manuscript_synctex: "inverse_runtime_manuscript_synctex",
  detect_latex_toolchain: "detect_runtime_latex_toolchain",
  start_manuscript_compile: "start_runtime_manuscript_compile",
  is_manuscript_compile_current: "is_runtime_manuscript_compile_current",
  read_manuscript_compile_pdf: "read_runtime_manuscript_compile_pdf",
  cancel_manuscript_compile: "cancel_runtime_manuscript_compile",
  get_manuscript_compile_run: "get_runtime_manuscript_compile_run",
  get_latest_manuscript_compile_run: "get_latest_runtime_manuscript_compile_run",
  get_latest_pdf_manuscript_compile_run: "get_latest_pdf_runtime_manuscript_compile_run",
  list_manuscripts: "list_runtime_manuscripts",
  create_manuscript: "create_runtime_manuscript",
  archive_manuscript: "archive_runtime_manuscript",
  restore_manuscript: "restore_runtime_manuscript",
  delete_manuscript: "delete_runtime_manuscript",
  update_manuscript: "update_runtime_manuscript",
  list_manuscript_files: "list_runtime_manuscript_files",
  regenerate_manuscript_bibliography: "restore_runtime_manuscript_bibliography",
  preserve_manuscript_bibliography_edits: "restore_runtime_manuscript_bibliography",
  list_manuscript_bibliography_entries: "list_runtime_manuscript_bibliography_entries",
  sync_manuscript_bibliography: "sync_runtime_manuscript_bibliography",
  rekey_manuscript_citations: "rekey_runtime_manuscript_citations",
  bind_manuscript_citation: "bind_runtime_manuscript_citation",
  search_manuscript_citations: "search_runtime_manuscript_citations",
  update_manuscript_bibliography_entry: "update_runtime_manuscript_bibliography_entry",
  link_manuscript_bibliography_entry: "link_runtime_manuscript_bibliography_entry",
  remove_unused_imported_bibliography_entries: "remove_unused_runtime_imported_bibliography_entries",
  list_manuscript_citation_bindings: "list_runtime_manuscript_citation_bindings",
  get_manuscript_bibliography_status: "get_runtime_manuscript_bibliography_status",
  read_manuscript_file: "read_runtime_manuscript_file",
  save_manuscript_file: "save_runtime_manuscript_file",
  create_manuscript_file: "create_runtime_manuscript_file",
  rename_manuscript_file: "rename_runtime_manuscript_file",
  delete_manuscript_file: "delete_runtime_manuscript_file",
  list_manuscript_directories: "list_runtime_manuscript_directories",
  create_manuscript_directory: "create_runtime_manuscript_directory",
  rename_manuscript_directory: "rename_runtime_manuscript_directory",
  delete_manuscript_directory: "delete_runtime_manuscript_directory",
  get_writing_session: "get_runtime_writing_session",
  save_writing_session: "save_runtime_writing_session",
  list_manuscript_recovery_drafts: "list_runtime_manuscript_recovery_drafts",
  save_manuscript_recovery_draft: "save_runtime_manuscript_recovery_draft",
  clear_manuscript_recovery_draft: "clear_runtime_manuscript_recovery_draft",
  query_writing_note_catalog: "query_runtime_writing_note_catalog",
  prepare_writing_note_catalog: "prepare_runtime_writing_note_catalog",
  save_writing_document_checkpoint: "save_runtime_writing_document_checkpoint",
  list_writing_document_checkpoints: "list_runtime_writing_document_checkpoints",
} as const;

function unavailable(code: string, message: string, packageKey: string): Error {
  return Object.assign(new Error(message), {code, packageKey, pluginId: "writing_projects"});
}

/** Bind the original Core-selected ports before the package's production roles
 * mount. The instance retains them until view/save cleanup has completed. */
export function bindRuntimeWritingWorkbench(plugin: LoadedRuntimePlugin, assertRetained: () => void): (() => void) | undefined {
  const {packageKey, manifest} = plugin;
  if (manifest.id !== "writing_projects" || !packageKey || !/^[0-9a-f]{64}$/u.test(packageKey) || manifest.native?.protocolVersion !== 2
    || !manifest.provides.some(capability => capability.id === "writing.project-files" && capability.version === 1)) return undefined;
  const key = packageKey;
  const runtime = getAppRuntime();
  if (runtime.mode === "browser" || runtime.mode === "preview" && !runtime.nativeAgentPreview
    || runtime.gateways.getWritingGateway !== undefined) return undefined;
  const transport = runtime.mode === "desktop" ? invokePlatform : runtime.invoke;
  if (!transport) return undefined;
  const dispatch = transport;
  let active = true;
  const current = (): void => {
    assertRetained();
    if (!active || getAppRuntime() !== runtime || runtime.gateways.getWritingGateway !== gateway) throw unavailable("writing_gateway_unavailable", "原写作适配已结束，请核对当前插件实例。", packageKey);
  };
  async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    current();
    if (!Object.hasOwn(projectCommands, command)) throw unavailable("writing_gateway_unavailable", `当前写作包尚未接入 ${command}。`, key);
    const target = projectCommands[command as keyof typeof projectCommands];
    try {return await dispatch(target, {...args, packageKey}) as T;}
    catch (reason) {throw nativeWritingOperationFailure(reason, key);}
  }
  const gateway = createTauriWritingGateway(invoke, async sourcePath => {
    current();
    if (!manifest.provides.some(capability => capability.id === "writing.import-preview" && capability.version === 1))
      throw unavailable("writing_gateway_unavailable", "当前写作包尚未提供导入预览。", packageKey);
    try {return await dispatch("preview_runtime_writing_project_import", {packageKey, sourcePath}) as Awaited<ReturnType<typeof gateway.previewImport>>;}
    catch (reason) {throw nativeWritingOperationFailure(reason, packageKey);}
  });
  const previousCheckpoint = runtime.gateways.getWritingCheckpointGateway;
  const checkpoints = previousCheckpoint ?? createTauriWritingCheckpointGateway(invoke);
  const previousAssets = runtime.gateways.getWritingNoteAssetGateway;
  const assets = runtime.mode === "desktop"
    ? previousAssets ?? createTauriWritingNoteAssetGateway(invoke)
    : undefined;
  runtime.gateways.getWritingGateway = gateway;
  runtime.gateways.getWritingCheckpointGateway = checkpoints;
  if (assets) runtime.gateways.getWritingNoteAssetGateway = assets;
  return () => {
    active = false;
    if (runtime.gateways.getWritingGateway === gateway) delete runtime.gateways.getWritingGateway;
    if (previousCheckpoint === undefined && runtime.gateways.getWritingCheckpointGateway === checkpoints)
      delete runtime.gateways.getWritingCheckpointGateway;
    if (previousAssets === undefined && runtime.gateways.getWritingNoteAssetGateway === assets)
      delete runtime.gateways.getWritingNoteAssetGateway;
  };
}
