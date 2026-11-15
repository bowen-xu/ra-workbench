// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {decodePluginManifest, hasPluginFrontend, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {RuntimePluginModule} from "@research-assistant/workbench-sdk/activation";
import {parseManuscriptPath} from "@research-assistant/writing";
import type {WritingGateway} from "@research-assistant/data/writing";
import {emptyLiteratureDraft} from "@research-assistant/core/literature";
import type {LoadedRuntimePlugin} from "../../../core/runtime/plugins/loader";
import {selectAppRuntime, type AppRuntime} from "../../../core/runtime/appRuntime";
import {getWritingGateway} from "@research-assistant/writing-projects-plugin/writingGateway";
import {getWritingCheckpointGateway} from "@research-assistant/writing-projects-plugin/writingCheckpointGateway";
import {getWritingNoteAssetNativeGateway} from "../../../core/public/writingRuntime";
import {createRuntimePluginWorkbench} from "../plugins/runtimePluginWorkbench";
import checkpointFixture from "../../../../../../packages/writing/fixtures/document-checkpoint-v1.json";

const native = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: native.invoke}));
const key = "b".repeat(64), manuscriptId = "11111111-1111-4111-8111-111111111111";
const workbenches: ReturnType<typeof createRuntimePluginWorkbench>[] = [];
afterEach(async () => {
  for (const workbench of workbenches.splice(0)) await workbench.dispose();
  native.invoke.mockReset();
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});
});

it("binds the original three asset methods to the retained native package", async () => {
  await setup();
  const assets = getWritingNoteAssetNativeGateway();
  const imported = {assetId: manuscriptId, displayName: "image.png", mimeType: "image/png"};
  const bytes = [0, 255, 42];
  const exported = {destinationPath: "/synthetic/image.png", fileCount: 1, byteSize: 3};
  native.invoke.mockResolvedValueOnce(imported).mockResolvedValueOnce(bytes).mockResolvedValueOnce(exported);
  const request = {manuscriptId, displayName: "image", mimeType: "image/png", bytes};
  await expect(assets.importAsset(request)).resolves.toBe(imported);
  await expect(assets.readAsset(manuscriptId, manuscriptId)).resolves.toBe(bytes);
  const destination = {manuscriptId, assetId: manuscriptId, destinationPath: "/synthetic/image.png"};
  await expect(assets.exportAsset(destination)).resolves.toBe(exported);
  expect(native.invoke.mock.calls).toEqual([
    ["import_runtime_writing_note_asset", {...request, packageKey: key}],
    ["read_runtime_writing_note_asset", {manuscriptId, assetId: manuscriptId, packageKey: key}],
    ["export_runtime_writing_note_asset", {...destination, packageKey: key}],
  ]);
});

it("releases the asset binding with its original instance and rejects captured calls", async () => {
  const {workbench, selected} = await setup();
  const assets = getWritingNoteAssetNativeGateway();
  await workbench.deactivate("writing_projects");
  expect(selected.gateways.getWritingNoteAssetGateway).toBeUndefined();
  await expect(assets.readAsset(manuscriptId, manuscriptId)).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  expect(native.invoke).not.toHaveBeenCalled();
  await workbench.activate({plugins: [plugin()], bindings: [], unavailableOptional: []});
  expect(getWritingNoteAssetNativeGateway()).not.toBe(assets);
});

it("preserves a selected asset override and a later replacement at disposal", async () => {
  const assets = {importAsset: vi.fn(), readAsset: vi.fn(), exportAsset: vi.fn()};
  const selected: AppRuntime = {mode: "desktop", storage: localStorage, gateways: {getWritingNoteAssetGateway: assets}, dispose() {}};
  const {workbench} = await setup(plugin(), selected);
  expect(getWritingNoteAssetNativeGateway()).toBe(assets);
  const replacement = {...assets};
  selected.gateways.getWritingNoteAssetGateway = replacement;
  await workbench.deactivate("writing_projects");
  expect(selected.gateways.getWritingNoteAssetGateway).toBe(replacement);
});

it("keeps native preview's desktop-only asset boundary", async () => {
  const selected = {mode: "preview" as const, storage: localStorage, gateways: {}, nativeAgentPreview: {} as NonNullable<AppRuntime["nativeAgentPreview"]>, invoke: native.invoke, dispose() {}};
  await setup(plugin(), selected);
  expect(() => getWritingNoteAssetNativeGateway()).toThrow(expect.objectContaining({code: "writing_note_asset_gateway_unavailable"}));
  expect(native.invoke).not.toHaveBeenCalled();
});
function plugin(activate: RuntimePluginModule["activate"] = () => {}): LoadedRuntimePlugin {
  const manifest = decodePluginManifest({formatVersion: 2, id: "writing_projects", version: "1.0.0", apiVersion: 1,
      entry: "entry.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
      native: {protocolVersion: 2, os: "macos", arch: "aarch64", entry: "native/writing-host"},
      dependencies: {required: [], optional: []}, consumes: {required: [], optional: []},
      provides: [{id: "writing.project-files", version: 1}, {id: "writing.import-preview", version: 1}]});
  if (!hasPluginFrontend(manifest)) throw new Error("fixture frontend required");
  return {packageKey: key, entryUrl: `/plugins/${"a".repeat(64)}/entry.js`, styles: [], module: {activate}, manifest};
}
async function setup(loaded = plugin(), selected: AppRuntime = {mode: "desktop", storage: localStorage, gateways: {}, dispose() {}}) {
  selectAppRuntime(selected);
  const workbench = createRuntimePluginWorkbench({storage: localStorage});
  workbenches.push(workbench);
  await workbench.activate({plugins: [loaded], bindings: [], unavailableOptional: []});
  return {workbench, selected};
}

const draft = {title: "Synthetic", rootPath: "main.tex"};
const update = {manuscriptId, title: "Changed", rootPath: parseManuscriptPath("main.tex")};
const save = {manuscriptId, relativePath: parseManuscriptPath("main.tex"), content: "Source", expectedSha256: "a".repeat(64)};
const session = {manuscriptId, openPaths: [], cursorState: {}, layoutState: {}};
const recovery = {manuscriptId, relativePath: parseManuscriptPath("main.tex"), baseSha256: "a".repeat(64), content: "Unsaved"};
const bibliographyDraft = {...emptyLiteratureDraft(), title: "Changed title"};
const binding = {literatureItemId: "source", requestedKey: "key", bibtexEntry: "@article{key,title={Title}}"};
const methods: readonly {call: (gateway: WritingGateway) => Promise<unknown>; command: string; args: Record<string, unknown>}[] = [
  {call: g => g.importProject("Imported", "/synthetic/project.zip", "main.tex"), command: "import_runtime_manuscript_project", args: {title: "Imported", sourcePath: "/synthetic/project.zip", rootPath: "main.tex"}},
  {call: g => {
    if (!g.duplicateManuscript) throw new Error("The original Tauri adapter must expose duplicateManuscript");
    return g.duplicateManuscript(manuscriptId);
  }, command: "duplicate_runtime_manuscript", args: {manuscriptId}},
  {call: g => g.exportSource(manuscriptId, "/synthetic/project.zip"), command: "export_runtime_manuscript_source", args: {manuscriptId, destinationPath: "/synthetic/project.zip", renameOnConflict: true}},
  {call: g => g.exportPdf(manuscriptId, "/synthetic/project.pdf"), command: "export_runtime_manuscript_pdf", args: {manuscriptId, destinationPath: "/synthetic/project.pdf", renameOnConflict: true}},
  {call: g => g.importResources(manuscriptId, "figures", ["/synthetic/refs.bib"]), command: "import_runtime_manuscript_resources", args: {manuscriptId, destinationDirectory: "figures", sourcePaths: ["/synthetic/refs.bib"]}},
  {call: g => g.forwardSynctex("run", "main.tex", 4, 0), command: "forward_runtime_manuscript_synctex", args: {runId: "run", relativePath: "main.tex", line: 4, column: 0}},
  {call: g => g.inverseSynctex("run", 2, 1.5, 3), command: "inverse_runtime_manuscript_synctex", args: {runId: "run", page: 2, x: 1.5, y: 3}},
  {call: g => g.detectToolchain(), command: "detect_runtime_latex_toolchain", args: {}},
  {call: g => g.startCompile(manuscriptId, "pdflatex", {draft: true, stopOnFirstError: false, fromScratch: true}), command: "start_runtime_manuscript_compile", args: {manuscriptId, engine: "pdflatex", options: {draft: true, stopOnFirstError: false, fromScratch: true}}},
  {call: g => g.getCompileRun("run"), command: "get_runtime_manuscript_compile_run", args: {runId: "run"}},
  {call: g => g.getLatestCompileRun(manuscriptId), command: "get_latest_runtime_manuscript_compile_run", args: {manuscriptId}},
  {call: g => g.getLatestPdfCompileRun(manuscriptId), command: "get_latest_pdf_runtime_manuscript_compile_run", args: {manuscriptId}},
  {call: g => g.isCompileCurrent("run"), command: "is_runtime_manuscript_compile_current", args: {runId: "run"}},
  {call: g => g.cancelCompile("run"), command: "cancel_runtime_manuscript_compile", args: {runId: "run"}},
  {call: g => g.readCompilePdf("run"), command: "read_runtime_manuscript_compile_pdf", args: {runId: "run"}},
  {call: g => g.createManuscript(draft), command: "create_runtime_manuscript", args: {draft}},
  {call: g => g.updateManuscript(update), command: "update_runtime_manuscript", args: {request: update}},
  {call: g => g.saveFile(save), command: "save_runtime_manuscript_file", args: {request: save}},
  {call: g => g.saveSession(session), command: "save_runtime_writing_session", args: {draft: session}},
  {call: g => g.saveRecoveryDraft(recovery), command: "save_runtime_manuscript_recovery_draft", args: {draft: recovery}},
  {call: g => g.listManuscripts(), command: "list_runtime_manuscripts", args: {}},
  {call: g => g.listFiles(manuscriptId), command: "list_runtime_manuscript_files", args: {manuscriptId}},
  {call: g => g.listBibliographyEntries(manuscriptId), command: "list_runtime_manuscript_bibliography_entries", args: {manuscriptId, includeUnusedAutomatic: false}},
  {call: g => g.listBibliographyEntries(manuscriptId, true), command: "list_runtime_manuscript_bibliography_entries", args: {manuscriptId, includeUnusedAutomatic: true}},
  {call: g => g.listCitationBindings(manuscriptId), command: "list_runtime_manuscript_citation_bindings", args: {manuscriptId}},
  {call: g => g.getBibliographyStatus(manuscriptId), command: "get_runtime_manuscript_bibliography_status", args: {manuscriptId}},
  {call: g => g.regenerateBibliography(manuscriptId), command: "restore_runtime_manuscript_bibliography", args: {manuscriptId}},
  {call: g => g.preserveBibliographyEdits(manuscriptId), command: "restore_runtime_manuscript_bibliography", args: {manuscriptId}},
  {call: g => g.syncBibliography(manuscriptId, []), command: "sync_runtime_manuscript_bibliography", args: {manuscriptId, entries: []}},
  {call: g => g.rekeyCitations(manuscriptId, []), command: "rekey_runtime_manuscript_citations", args: {manuscriptId, assignments: []}},
  {call: g => g.bindCitation(manuscriptId, binding), command: "bind_runtime_manuscript_citation", args: {manuscriptId, entry: binding}},
  {call: g => g.updateBibliographyEntry(manuscriptId, "entry", "expected", bibliographyDraft), command: "update_runtime_manuscript_bibliography_entry", args: {manuscriptId, entryId: "entry", expectedBibtex: "expected", draft: bibliographyDraft}},
  {call: g => g.linkBibliographyEntry(manuscriptId, "entry", "source"), command: "link_runtime_manuscript_bibliography_entry", args: {manuscriptId, entryId: "entry", literatureItemId: "source"}},
  {call: g => g.removeUnusedImportedBibliographyEntries(manuscriptId), command: "remove_unused_runtime_imported_bibliography_entries", args: {manuscriptId}},
  {call: g => g.readFile(manuscriptId, "main.tex"), command: "read_runtime_manuscript_file", args: {manuscriptId, relativePath: "main.tex"}},
  {call: g => g.createFile(manuscriptId, "extra.tex"), command: "create_runtime_manuscript_file", args: {manuscriptId, relativePath: "extra.tex", content: ""}},
  {call: g => g.renameFile(manuscriptId, "a.tex", "b.tex"), command: "rename_runtime_manuscript_file", args: {manuscriptId, fromPath: "a.tex", toPath: "b.tex"}},
  {call: g => g.deleteFile(manuscriptId, "extra.tex"), command: "delete_runtime_manuscript_file", args: {manuscriptId, relativePath: "extra.tex"}},
  {call: g => g.listDirectories(manuscriptId), command: "list_runtime_manuscript_directories", args: {manuscriptId}},
  {call: g => g.createDirectory(manuscriptId, "chapters"), command: "create_runtime_manuscript_directory", args: {manuscriptId, relativePath: "chapters"}},
  {call: g => g.renameDirectory(manuscriptId, "a", "b"), command: "rename_runtime_manuscript_directory", args: {manuscriptId, fromPath: "a", toPath: "b"}},
  {call: g => g.deleteDirectory(manuscriptId, "chapters"), command: "delete_runtime_manuscript_directory", args: {manuscriptId, relativePath: "chapters"}},
  {call: g => g.archiveManuscript(manuscriptId), command: "archive_runtime_manuscript", args: {manuscriptId}},
  {call: g => g.restoreManuscript(manuscriptId), command: "restore_runtime_manuscript", args: {manuscriptId}},
  {call: g => g.deleteManuscript(manuscriptId), command: "delete_runtime_manuscript", args: {manuscriptId}},
  {call: g => g.getSession(manuscriptId), command: "get_runtime_writing_session", args: {manuscriptId}},
  {call: g => g.listRecoveryDrafts(manuscriptId), command: "list_runtime_manuscript_recovery_drafts", args: {manuscriptId}},
  {call: g => g.clearRecoveryDraft(manuscriptId, "main.tex"), command: "clear_runtime_manuscript_recovery_draft", args: {manuscriptId, relativePath: "main.tex"}},
  {call: g => g.previewImport("/synthetic/project.zip"), command: "preview_runtime_writing_project_import", args: {sourcePath: "/synthetic/project.zip"}},
];
it.each(methods)("uses the discovered key for $command through the existing Core resolver", async ({call, command, args}) => {
  await setup(); native.invoke.mockResolvedValueOnce(undefined);
  await call(getWritingGateway());
  expect(native.invoke).toHaveBeenCalledExactlyOnceWith(command, {...args, packageKey: key});
});

it("keeps the existing nullable citation decoder on the package-bound search route", async () => {
  await setup();
  native.invoke.mockResolvedValueOnce([{literatureItemId: "source", title: "Title", issuedYear: null, citationKey: null}]);
  await expect(getWritingGateway().searchCitations(manuscriptId, "query", "library")).resolves.toEqual([{literatureItemId: "source", title: "Title"}]);
  expect(native.invoke).toHaveBeenCalledExactlyOnceWith("search_runtime_manuscript_citations", {manuscriptId, query: "query", scope: "library", packageKey: key});
});

it.each(["compile_run_not_found", "invalid_engine", "toolchain_unavailable", "synctex_unavailable", "synctex_no_result", "compile_process_error"])("retains the original compile cause and identity for %s", async code => {
  await setup();
  native.invoke.mockRejectedValueOnce({code, message: "原编译或定位原因", packageKey: key,
    instanceId: "compile-instance", requestId: "compile-request", privateBody: "PRIVATE_BODY"});
  const pending = getWritingGateway().forwardSynctex("run", "main.tex", 1, 0);
  await expect(pending).rejects.toMatchObject({code, packageKey: key, instanceId: "compile-instance",
    requestId: "compile-request", message: expect.stringContaining("原编译或定位原因")});
  await expect(pending).rejects.not.toThrow("PRIVATE_BODY");
});

it.each(["invalid_bibtex_entry", "bibliography_import", "bibliography_conflict", "literature_citations_unavailable", "citation_source_not_found"])("retains the bibliography rejection %s and actual host identity", async code => {
  await setup();
  native.invoke.mockRejectedValueOnce({code, message: "书目输入或来源需要核对", packageKey: key,
    instanceId: "bibliography-instance", requestId: "bibliography-request"});
  await expect(getWritingGateway().listBibliographyEntries(manuscriptId)).rejects.toMatchObject({
    code, packageKey: key, instanceId: "bibliography-instance", requestId: "bibliography-request",
    message: expect.stringContaining("书目输入或来源需要核对"),
  });
});

it("binds before activation and retains one observed gateway and checkpoint codec", async () => {
  let activated: WritingGateway | undefined;
  const {selected} = await setup(plugin(() => {activated = getWritingGateway();}));
  expect(activated).toBe(getWritingGateway());
  expect(selected.gateways.getWritingGateway).toBeDefined();
  const write = {expectedRevision: null, checkpoint: {...checkpointFixture, schemaVersion: 1 as const}};
  const saved = {revision: 1, checkpoint: write.checkpoint};
  native.invoke.mockResolvedValueOnce(saved).mockResolvedValueOnce([saved]);
  const checkpoints = getWritingCheckpointGateway();
  await expect(checkpoints.save(write)).resolves.toEqual(saved);
  await expect(checkpoints.list(write.checkpoint.manuscriptId)).resolves.toEqual([saved]);
  expect(native.invoke.mock.calls).toEqual([
    ["save_runtime_writing_document_checkpoint", {write, packageKey: key}],
    ["list_runtime_writing_document_checkpoints", {manuscriptId: write.checkpoint.manuscriptId, packageKey: key}],
  ]);
  native.invoke.mockResolvedValueOnce([{revision: 1, checkpoint: {...write.checkpoint, manuscriptId: "other"}}]);
  await expect(checkpoints.list(write.checkpoint.manuscriptId)).rejects.toThrow("不匹配");
});

it("uses the original note catalog codecs and preserves current revision diagnostics", async () => {
  await setup();
  const catalog = getWritingGateway().noteCatalog!;
  const page = {items: [], revision: 3, pendingCount: 0, errorCount: 0, issues: []};
  const query = {category: "all", view: "recent"} as const;
  native.invoke.mockResolvedValueOnce(page).mockResolvedValueOnce({processedCount: 1, pendingCount: 0, errorCount: 0});
  await expect(catalog.queryWritingNoteCatalog(query)).resolves.toEqual(page);
  await catalog.prepareWritingNoteCatalog({scope: "archived", retryErrors: true});
  expect(native.invoke.mock.calls).toEqual([
    ["query_runtime_writing_note_catalog", {query, packageKey: key}],
    ["prepare_runtime_writing_note_catalog", {scope: "archived", retryErrors: true, packageKey: key}],
  ]);
  native.invoke.mockRejectedValueOnce({code: "revision_conflict", message: "note_catalog_changed", currentRevision: 19});
  await expect(catalog.queryWritingNoteCatalog(query)).rejects.toMatchObject({code: "revision_conflict", currentRevision: 19, message: expect.stringContaining("note_catalog_changed")});
  native.invoke.mockResolvedValueOnce({...page, body: "PRIVATE"});
  await expect(catalog.queryWritingNoteCatalog(query)).rejects.toThrow("writing_note_catalog_response_invalid");
});

it("releases successful bindings and rejects calls captured from a retired instance", async () => {
  const {workbench, selected} = await setup();
  const gateway = getWritingGateway(), checkpoints = getWritingCheckpointGateway();
  await workbench.deactivate("writing_projects");
  expect(selected.gateways.getWritingGateway).toBeUndefined();
  expect(selected.gateways.getWritingCheckpointGateway).toBeUndefined();
  await expect(gateway.listManuscripts()).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  await expect(checkpoints.list(manuscriptId)).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  expect(native.invoke).not.toHaveBeenCalled();
});

it("keeps the original binding during rejected close and its save preparation", async () => {
  const loaded = plugin(context => {context.registerView({id: "writing", title: "Writing", render: () => null});});
  const {workbench, selected} = await setup(loaded);
  const gateway = getWritingGateway(); let allowed = false;
  workbench.attachNavigation({open: async () => {}, prepareClose: async () => {
    native.invoke.mockResolvedValueOnce(undefined);
    await gateway.clearRecoveryDraft(manuscriptId, "main.tex");
    return allowed;
  }});
  await expect(workbench.deactivate("writing_projects")).rejects.toMatchObject({code: "plugin_deactivation_failed"});
  expect(getWritingGateway()).toBe(gateway);
  expect(selected.gateways.getWritingGateway).toBeDefined();
  allowed = true; await workbench.deactivate("writing_projects");
  expect(selected.gateways.getWritingGateway).toBeUndefined();
  expect(native.invoke).toHaveBeenCalledTimes(2);
});

it("keeps an accepted result with its original waiter after runtime replacement", async () => {
  const {workbench, selected} = await setup();
  const gateway = getWritingGateway(); let finish!: (value: unknown) => void;
  native.invoke.mockImplementationOnce(() => new Promise(resolve => {finish = resolve;}));
  const pending = gateway.listManuscripts();
  const next = {mode: "desktop" as const, storage: localStorage, gateways: {}, dispose() {}};
  selectAppRuntime(next);
  await expect(gateway.listManuscripts()).rejects.toMatchObject({code: "writing_gateway_unavailable"});
  const rows = [{id: manuscriptId}]; finish(rows);
  await expect(pending).resolves.toEqual(rows);
  await workbench.deactivate("writing_projects");
  expect(selected.gateways.getWritingGateway).toBeUndefined();
  expect(next.gateways).toEqual({});
  expect(native.invoke).toHaveBeenCalledTimes(1);
});

it("preserves explicit overrides and full mock authority", async () => {
  const override = getWritingGateway();
  const selected: AppRuntime = {mode: "preview", storage: localStorage, gateways: {getWritingGateway: override}, dispose() {}};
  const {workbench} = await setup(plugin(), selected);
  expect(selected.gateways.getWritingGateway).toBe(override);
  await workbench.deactivate("writing_projects");
  expect(selected.gateways.getWritingGateway).toBe(override);
  expect(native.invoke).not.toHaveBeenCalled();
});

it("uses the original selected native preview transport", async () => {
  const invoke = vi.fn(async () => []);
  const selected: AppRuntime = {mode: "preview", storage: localStorage, gateways: {}, invoke,
    nativeAgentPreview: {} as NonNullable<AppRuntime["nativeAgentPreview"]>, dispose() {}};
  await setup(plugin(), selected);
  await expect(getWritingGateway().listManuscripts()).resolves.toEqual([]);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("list_runtime_manuscripts", {packageKey: key});
  expect(native.invoke).not.toHaveBeenCalled();
});

it("retains safe native diagnostics through the finite writing gateway", async () => {
  await setup(); const gateway = getWritingGateway();
  const id = "22222222-2222-4222-8222-222222222222";
  native.invoke.mockRejectedValueOnce({code: "storage_error", message: "文件保存失败", instanceId: id, requestId: id,
    dataError: {code: "plugin_data_statement_failed", phase: "read", sqliteCode: "SystemIoFailure", sqliteExtendedCode: 778,
      body: "PRIVATE", sql: "PRIVATE_SQL"}, rawStderr: "PRIVATE_STDERR"});
  await expect(gateway.readFile(manuscriptId, "main.tex")).rejects.toMatchObject({code: "storage_error", packageKey: key,
    instanceId: id, requestId: id, message: expect.stringContaining("dataError.sqliteExtendedCode=778")});
});

it("cleans its binding after activation rejects and keeps other instance registrations intact", async () => {
  let captured: WritingGateway | undefined;
  const selected: AppRuntime = {mode: "desktop", storage: localStorage, gateways: {}, dispose() {}};
  selectAppRuntime(selected);
  const workbench = createRuntimePluginWorkbench({storage: localStorage}); workbenches.push(workbench);
  const loaded = plugin(() => {captured = getWritingGateway(); throw new Error("activation fixture");});
  await expect(workbench.activate({plugins: [loaded], bindings: [], unavailableOptional: []})).rejects.toMatchObject({code: "plugin_activation_failed"});
  expect(selected.gateways.getWritingGateway).toBeUndefined();
  await expect(captured!.listManuscripts()).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  expect(native.invoke).not.toHaveBeenCalled();
});

it("rejects an old port after explicit gateway replacement and preserves the replacement", async () => {
  const {workbench, selected} = await setup();
  const captured = getWritingGateway();
  const replacement = {...selected.gateways.getWritingGateway!};
  selected.gateways.getWritingGateway = replacement;
  await expect(captured.listManuscripts()).rejects.toMatchObject({code: "writing_gateway_unavailable"});
  await workbench.deactivate("writing_projects");
  expect(selected.gateways.getWritingGateway).toBe(replacement);
  expect(native.invoke).not.toHaveBeenCalled();
});

it("reactivates using a new bound identity and leaves captured old methods closed", async () => {
  const loaded = plugin(); const {workbench} = await setup(loaded);
  const old = getWritingGateway();
  await workbench.deactivate("writing_projects");
  await workbench.activate({plugins: [loaded], bindings: [], unavailableOptional: []});
  const current = getWritingGateway(); expect(current).not.toBe(old);
  await expect(old.listManuscripts()).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  native.invoke.mockResolvedValueOnce([]);
  await expect(current.listManuscripts()).resolves.toEqual([]);
  expect(native.invoke).toHaveBeenCalledExactlyOnceWith("list_runtime_manuscripts", {packageKey: key});
});

it.each(["frontend-only", "legacy-preview", "missing-key", "other-owner"])("retains original missing capability semantics for %s", async kind => {
  const loaded = plugin();
  const selected: AppRuntime = {mode: "desktop", storage: localStorage, gateways: {}, dispose() {}};
  const original = loaded.manifest;
  const manifest = decodePluginManifest({...original,
    ...(kind === "frontend-only" ? {formatVersion: 1, native: undefined} : {}),
    ...(kind === "legacy-preview" ? {native: {...original.native!, protocolVersion: 1}, provides: [{id: "writing.import-preview", version: 1}]} : {}),
    ...(kind === "other-owner" ? {id: "note_manager"} : {}),
  });
  if (!hasPluginFrontend(manifest)) throw new Error("fixture frontend required");
  const {packageKey: omitted, ...withoutKey} = loaded;
  expect(omitted).toBe(key);
  const candidate = kind === "missing-key" ? {...withoutKey, manifest} : {...loaded, manifest};
  await setup(candidate, selected);
  expect(selected.gateways.getWritingGateway).toBeUndefined();
  expect(native.invoke).not.toHaveBeenCalled();
});

it("keeps native file conflict and HTTP identity without including unrelated payload", async () => {
  await setup();
  const operationId = "22222222-2222-4222-8222-222222222222";
  native.invoke.mockRejectedValueOnce({code: "file_conflict", message: "文件已在其他位置被修改", operationId,
    status: 503, body: "PRIVATE_SOURCE", authorization: "PRIVATE_CREDENTIAL"});
  const error: unknown = await getWritingGateway().readFile(manuscriptId, "main.tex").catch(reason => reason);
  expect(error).toMatchObject({code: "file_conflict", operationId, status: 503,
    message: expect.stringContaining("文件已在其他位置被修改")});
  expect((error as Error).message).not.toMatch(/PRIVATE|authorization/u);
});
