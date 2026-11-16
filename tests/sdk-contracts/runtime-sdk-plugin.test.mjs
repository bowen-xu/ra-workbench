import {copySdkArtifact} from "./sdk-artifacts.mjs";
import assert from "node:assert/strict";
import {afterEach, test} from "node:test";
import path from "node:path";
import {createRequire} from "node:module";
import {fileURLToPath, pathToFileURL} from "node:url";
import {mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {execFileSync, spawnSync} from "node:child_process";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const require = createRequire(path.join(root, "ra-workbench/apps/web/package.json"));
const ts = require("typescript");
const {build, loadConfigFromFile} = await import(pathToFileURL(require.resolve("vite")).href);
const temporary = [];
const writingExistingExtensions = {
  "core-notes": ["blocksToEditor", "decodeNoteBlocks", "editorToBlocks", "isAllowedNoteHref", "noteImageDisplayName"],
  feedback: ["NotificationEffect"],
  "note-documents": ["NotePageToolbar", "openNoteDocumentTab"],
  platform: ["downloadDir", "join"],
  "source-navigation": ["openSourceLink"],
  "workspace-state": ["workspaceBoolean"],
  "domain-capabilities": ["getOptionalLiteratureGateway"],
  environment: ["getRuntimeStorage", "getNativePreviewCapabilities", "captureNewPreviewOwnerIntent", "freezeNewPreviewOwnerIntent"],
  storage: ["flushStorage", "storagePersistence"],
  ui: ["InlineReference"],
  pdf: ["PdfViewControls"],
  "document-recovery": ["EDITOR_AUTOSAVE_IDLE_MS"],
};
const writingCoreFacadeNames = ["block-content", "document-windows", "editor-review", "code-editor-extensions",
  "writing-storage", "writing-runtime", "git-versions", "git-worktrees", "text-diff-stats", "agent-gateways",
  "agent-delivery", "agent-diagnostics", "agent-drafts", "agent-writing", "agent-conversations", "agent-context-controls",
  "agent-submission", "agent-presentation", "agent-canvas"];
afterEach(() => {for (const directory of temporary.splice(0)) rmSync(directory, {recursive: true, force: true});});
async function load(runtime, selection, command = "build") {
  const keys = ["RESEARCH_ASSISTANT_RUNTIME_PLUGINS", "RESEARCH_ASSISTANT_BUILTIN_PLUGINS"];
  const previous = keys.map(key => process.env[key]);
  for (const [index, value] of [runtime, selection].entries()) {
    if (value === undefined) delete process.env[keys[index]]; else process.env[keys[index]] = value;
  }
  try {return await loadConfigFromFile({command, mode: command === "build" ? "production" : "development"}, path.join(root, "ra-workbench/apps/web/vite.config.ts"), path.join(root, "ra-workbench/apps/web"), "silent");}
  finally {keys.forEach((key, index) => {if (previous[index] === undefined) delete process.env[key]; else process.env[key] = previous[index];});}
}

test("legacy selection variables cannot add static plugin implementations", async () => {
  for (const selection of ["none", "note_manager", "all"]) {
    const loaded = await load(undefined, selection);
    const plugins = loaded.config.plugins.flat(Infinity);
    assert.ok(plugins.some(plugin => plugin?.name === "workbench-runtime-sdk"));
    assert.deepEqual(plugins.find(plugin => plugin?.name === "workbench-runtime-core").api.selection.plugins, []);
  }
});

test("the ordinary Web build selects the same empty Core and shared SDK entries", async () => {
  const loaded = await load(undefined, undefined);
  const plugins = loaded.config.plugins.flat(Infinity);
  const composition = plugins.find(plugin => plugin?.name === "workbench-runtime-core");
  const sdk = plugins.find(plugin => plugin?.name === "workbench-runtime-sdk");
  assert.ok(sdk);
  assert.deepEqual(composition.api.selection.plugins, []);
  assert.deepEqual(composition.api.selection.cargoFeatures, []);
  assert.equal(loaded.config.define.__RUNTIME_PLUGINS__, true);
  composition.configResolved({command: "build"});
  assert.match(composition.transformIndexHtml.handler('<script src="/src/main.tsx"></script>'), /main\.selected\.tsx/u);
  for (const output of ["dist", path.join(root, "ra-workbench/.tmp/normal-core-output")])
    assert.doesNotThrow(() => sdk.configResolved({root: path.join(root, "ra-workbench/apps/web"), build: {outDir: output}}));
  assert.throws(() => sdk.configResolved({root: path.join(root, "ra-workbench/apps/web"), build: {outDir: "../foreign-output"}}), /runtime_sdk_output_invalid/u);
});

test("ordinary browser development retains its existing production component harness", async () => {
  const loaded = await load(undefined, undefined, "serve");
  const plugins = loaded.config.plugins.flat(Infinity);
  assert.ok(!plugins.some(plugin => plugin?.name === "workbench-runtime-sdk"));
  const composition = plugins.find(plugin => plugin?.name === "workbench-runtime-core");
  assert.deepEqual(composition.api.selection.plugins, []);
  const html = '<script src="/src/main.tsx"></script>';
  assert.equal(composition.transformIndexHtml.handler(html), html);
  for (const id of ["literature_library", "writing_projects"]) {
    const browser = await loadConfigFromFile({command: "serve", mode: "development"}, path.join(root, "integration/web/vite.config.ts"), path.join(root, "integration/web"), "silent");
    assert.equal(browser.config.resolve.alias[`/plugins/${id}/src`], path.join(root, "plugins", {literature_library: "ra-literature-library", writing_projects: "ra-writing-projects"}[id], "src"));
  }
});

test("Core HTML loads original shared PDF styles even when only a separate SDK entry imports them", async () => {
  const loaded = await load("1", "none");
  const plugin = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk");
  const policy = plugin.config({}, {command: "build", mode: "production"}).build;
  const directory = mkdtempSync(path.join(root, ".tmp/runtime-sdk-css-"));
  temporary.push(directory);
  writeFileSync(path.join(directory, "index.html"), '<div id="root"></div><script type="module" src="/main.js"></script>');
  writeFileSync(path.join(directory, "main.js"), 'document.getElementById("root").textContent = "Core";');
  const styles = ["components/pdf-view/PdfView.css", "features/dictionary/DictionaryLookup.css", "components/hbs/HbsDocumentEditor.css",
    "public/documentWindows.css", "public/agentChanges.css"];
  writeFileSync(path.join(directory, "pdf.js"), styles.map(file =>
    `import ${JSON.stringify(path.join(root, "ra-workbench/apps/web/src/core", file))};`).join("\n") + "\nexport const shared = true;");
  const output = await build({configFile: false, root: directory, logLevel: "silent", build: {
    ...policy, write: false,
    rolldownOptions: {...policy.rolldownOptions, input: {
      index: path.join(directory, "index.html"), "sdk/v1/pdf": path.join(directory, "pdf.js"),
    }},
  }});
  const artifacts = (Array.isArray(output) ? output : [output]).flatMap(value => value.output);
  const html = String(artifacts.find(value => value.fileName === "index.html").source);
  const pdfStyles = artifacts.filter(value => value.type === "asset" && value.fileName.endsWith(".css")
    && String(value.source).includes(".pdf-view.pdf-viewer-container"));
  assert.equal(pdfStyles.length, 1, "the real Core stylesheet must be emitted once");
  assert.ok(html.includes(`href="/${pdfStyles[0].fileName}"`), "raw ESM import cannot fetch extracted CSS; Core HTML must load it");
  assert.match(String(pdfStyles[0].source), /\.pdf-view\.pdf-viewer-container\s*\{[^}]*position:\s*absolute/u);
  assert.ok(String(pdfStyles[0].source).includes(".dictionary-lookup"));
  assert.ok(String(pdfStyles[0].source).includes(".hbs-note-editor"));
  assert.ok(String(pdfStyles[0].source).includes(".reading-note-document-header"));
  assert.ok(String(pdfStyles[0].source).includes(".writing-agent-inline-diff"));
  for (const [producer, stylesheet] of [["documentWindows", "documentWindows"], ["agentChanges", "agentChanges"]])
    assert.match(readFileSync(path.join(root, "ra-workbench/apps/web/src/core/public", producer + ".ts"), "utf8"),
      new RegExp(`import "\\./${stylesheet}\\.css";`, "u"));
});

test("the real provider plan covers the metadata reexports and one shared UI runtime", async () => {
  const loaded = await load("1", "none");
  const plugin = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk");
  const entries = plugin.api.facades;
  assert.equal(new Set(entries.map(value => value.name)).size, entries.length);
  for (const name of ["NOTE_METADATA_CHANGED", "NOTE_PAGES_DELETED", "CoreNoteWorkspaceHost", "useCoreNoteWorkspaceHost"]) assert.ok(entries.find(value => value.name === "note-documents").exports.includes(name));
  for (const name of ["applyNotePageStatus", "publishNoteMetadataReceipt"]) assert.ok(entries.find(value => value.name === "note-resources").exports.includes(name));
  assert.deepEqual(entries.find(value => value.name === "platform").exports, ["chooseAppFile", "confirm", "open", ...writingExistingExtensions.platform]);
  assert.equal(entries.find(value => value.name === "react").source, "react");
  assert.equal(entries.find(value => value.name === "react-jsx-runtime").source, "react/jsx-runtime");
  assert.ok(entries.filter(value => value.source.startsWith("apps/")).every(value => value.source.startsWith("apps/web/src/core/public/")));
});

test("the finite literature increment extends only the six original Core facades", async () => {
  const loaded = await load("1", "none");
  const entries = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk").api.facades;
  const additions = {
    feedback: ["useNotificationState"],
    "note-resources": ["useNoteAnnotations", "useLiteratureNotes"],
    overlay: ["OverlayPortal", "Popover", "Dialog", "useOverlayTarget"],
    platform: ["open"],
    reorder: ["ReorderableListRow", "ReorderDragHandle"],
    workbench: ["usePublishWorkspaceTabs"],
  };
  const frozen = JSON.parse(readFileSync(path.join(root, "docs/architecture/runtime-note-sdk-surface.json"), "utf8"));
  assert.equal(frozen.modules.reduce((count, value) => count + value.runtime.length, 0), 74);
  assert.deepEqual(frozen.runtimeExtensions.flatMap(value => value.runtime), ["CoreNoteWorkspaceHost", "useCoreNoteWorkspaceHost", "confirm"]);
  const metadataReexports = {
    "note-documents": ["NOTE_METADATA_CHANGED", "NOTE_PAGES_DELETED"],
    "note-resources": ["applyNotePageStatus", "publishNoteMetadataReceipt"],
  };
  const coreEntries = entries.filter(value => frozen.modules.some(original =>
    value.source === "apps/web/src/" + original.module + ".ts"));
  assert.equal(coreEntries.length, frozen.modules.filter(value => value.module.startsWith("core/public/") && value.runtime.length).length);
  for (const facade of coreEntries) {
    const module = facade.source.slice("apps/web/src/".length, -".ts".length);
    const original = frozen.modules.find(value => value.module === module);
    assert.ok(original, "no new Core facade subpath: " + facade.name);
    assert.deepEqual(facade.exports, [...new Set([...original.runtime, ...(metadataReexports[facade.name] ?? []),
      ...frozen.runtimeExtensions.filter(value => value.module === module).flatMap(value => value.runtime),
      ...(additions[facade.name] ?? []), ...(writingExistingExtensions[facade.name] ?? [])])]);
  }
  for (const [name, values] of Object.entries(additions)) {
    const facade = entries.find(value => value.name === name);
    assert.equal(facade.source, "apps/web/src/core/public/" + ({"note-resources": "noteResources"}[name] ?? name) + ".ts");
    for (const value of values) assert.ok(facade.exports.includes(value), name + ": " + value);
  }
  assert.deepEqual(entries.find(value => value.name === "core-notes").exports, [
    ...frozen.modules.find(value => value.module === "@research-assistant/core/notes").runtime,
    "isPositionReadingNote", "readingNoteSourceAt", ...writingExistingExtensions["core-notes"],
  ]);
});

test("the three new literature SDK leaves bind only their frozen Core public values", async () => {
  const loaded = await load("1", "none");
  const entries = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk").api.facades;
  const frozen = JSON.parse(readFileSync(path.join(root, "docs/architecture/runtime-note-sdk-surface.json"), "utf8"));
  const additions = [
    {name: "environment", source: "apps/web/src/core/public/pluginRuntimeEnvironment.ts", exports: ["isAppPreview", "isDesktopRuntime", "getRuntimeEnvironment", "ensureRuntimeStructuredResources", "hasPlatformCapabilities", ...writingExistingExtensions.environment]},
    {name: "shortcuts", source: "apps/web/src/core/public/settings.ts", exports: ["formatShortcut", "isShortcutMac"]},
    {name: "settings", source: "apps/web/src/core/public/settings.ts", exports: ["SettingRow", "SettingGroup", "useDeveloperMode", "SystemDocumentDialog"]},
  ];
  const newCoreEntries = entries.filter(value => additions.some(addition => addition.name === value.name));
  assert.deepEqual(newCoreEntries, additions);
  assert.ok(newCoreEntries.every(value => !frozen.modules.some(original => value.source === "apps/web/src/" + original.module + ".ts")));
  for (const facade of additions) {
    const bound = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk");
    const code = bound.load("\0virtual:workbench-runtime-sdk/" + facade.name);
    assert.ok(code.startsWith("export {" + facade.exports.join(",") + "} from "));
    assert.ok(code.includes(facade.source));
  }
});

test("the independent Agent UI and block-control leaves retain their exact public bindings", async () => {
  const loaded = await load("1", "none");
  const entries = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk").api.facades;
  const additions = [
    {name: "agent-ui", source: "apps/web/src/core/public/pluginAgentUi.ts", exports: ["SelectionAskAiButton", "SelectionAskAiEditor", "submitEvidenceAnnotation", "openAgentConversation", "useAgentCanvasMessage", "captureEvidenceImage"]},
    {name: "block-controls", source: "apps/web/src/core/public/blockControls.ts", exports: ["BlockControlsProvider", "FloatingBlockControls", "NoteActionMenu", "NoteActionMenuItem", "BlockAddButton", "BlockDragButton", "useBlockControlsRoot", "startBlockPointerDrag", "getEditorScrollContainer", "useBlockDragAutoScroll"]},
  ];
  assert.deepEqual(entries.filter(value => additions.some(addition => addition.name === value.name)), additions);
  const frozen = JSON.parse(readFileSync(path.join(root, "docs/architecture/runtime-note-sdk-surface.json"), "utf8"));
  const newCoreEntries = entries.filter(value => value.source.startsWith("apps/web/src/core/public/")
    && !frozen.modules.some(original => value.source === "apps/web/src/" + original.module + ".ts"));
  assert.deepEqual(newCoreEntries.map(value => value.name), ["environment", "storage", "literature-runtime", "shortcuts", "settings", "agent-ui", "block-controls",
    "ui", "structured-field-colors", "text-documents", "model-route-picker", "pdf", "pdf-geometry", "library-capture", "docling", "dictionary",
    "annotation-notes", "documents", "document-recovery", "hbs", "literature-notes", "structured-resources", "ai-assistance", "concept-retrieval",
    ...writingCoreFacadeNames]);
});

test("storage retains the original getter followed by the finite writing persistence values", async () => {
  const loaded = await load("1", "none");
  const plugins = loaded.config.plugins.flat(Infinity);
  const runtime = plugins.find(value => value?.name === "workbench-runtime-sdk");
  const source = plugins.find(value => value?.name === "workbench-sdk-source-bindings");
  assert.deepEqual(runtime.api.facades.find(value => value.name === "storage"), {
    name: "storage", source: "apps/web/src/core/public/storage.ts", exports: ["getAppStorage", ...writingExistingExtensions.storage],
  });
  const expected = `export {${["getAppStorage", ...writingExistingExtensions.storage].join(",")}} from ${JSON.stringify(path.join(root, "ra-workbench/apps/web/src/core/public/storage.ts"))};`;
  assert.equal(runtime.load("\0virtual:workbench-runtime-sdk/storage"), expected);
  assert.equal(source.load(source.resolveId("@research-assistant/workbench-sdk/storage")), expected);
  assert.deepEqual(runtime.api.facades.find(value => value.name === "environment").exports,
    ["isAppPreview", "isDesktopRuntime", "getRuntimeEnvironment", "ensureRuntimeStructuredResources", "hasPlatformCapabilities", ...writingExistingExtensions.environment]);
});

test("literature routing binds only the ten existing resolver values from the same Core source", async () => {
  const loaded = await load("1", "none");
  const plugins = loaded.config.plugins.flat(Infinity);
  const runtime = plugins.find(value => value?.name === "workbench-runtime-sdk");
  const source = plugins.find(value => value?.name === "workbench-sdk-source-bindings");
  const exports = ["resolveLiteratureRuntimeGateway", "resolveCollectionRuntimeGateway", "resolveNalRuntimeGateway", "resolveNalQueryRuntimeGateway",
    "resolveKnowledgeRuntimeGateway", "resolveEvidenceRuntimeGateway", "resolveEvidenceMatrixRuntimeGateway", "resolveDerivedViewRuntimeGateway",
    "resolveReferenceRecordRuntimeGateway", "resolveResearchExportRuntimeGateway"];
  assert.deepEqual(runtime.api.facades.find(value => value.name === "literature-runtime"), {
    name: "literature-runtime", source: "apps/web/src/core/public/literatureRuntime.ts", exports,
  });
  const expected = `export {${exports.join(",")}} from ${JSON.stringify(path.join(root, "ra-workbench/apps/web/src/core/public/literatureRuntime.ts"))};`;
  assert.equal(runtime.load("\0virtual:workbench-runtime-sdk/literature-runtime"), expected);
  assert.equal(source.load(source.resolveId("@research-assistant/workbench-sdk/literature-runtime")), expected);
  const id = path.join(root, "ra-workbench/apps/web/src/core/public/literatureRuntime.ts");
  const bundle = Object.fromEntries(runtime.api.facades.map(facade => [facade.name, {
    type: "chunk", fileName: "sdk/v1/" + facade.name + ".js", exports: facade.exports, imports: [], modules: {},
  }]));
  bundle["literature-runtime"].modules[id] = {};
  bundle.environment.modules[id] = {};
  const emitted = [];
  const context = {getModuleIds: () => [id][Symbol.iterator](),
    error: message => {throw new Error(message);}, emitFile: asset => emitted.push(asset)};
  assert.throws(() => runtime.generateBundle.call(context, {}, bundle), /runtime_sdk_owner_identity_violation/u);
  delete bundle.environment.modules[id];
  runtime.generateBundle.call(context, {}, bundle);
  assert.deepEqual(JSON.parse(emitted[0].source).owners, [{module: "apps/web/src/core/public/literatureRuntime.ts", chunks: ["sdk/v1/literature-runtime.js"]}]);
});

test("the existing owner guard preserves one selected Core storage module", async () => {
  const loaded = await load("1", "none");
  const plugin = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk");
  const id = path.join(root, "ra-workbench/apps/web/src/core/runtime/appStorage.ts");
  const bundle = Object.fromEntries(plugin.api.facades.map(facade => [facade.name, {
    type: "chunk", fileName: "sdk/v1/" + facade.name + ".js", exports: facade.exports, imports: [], modules: {},
  }]));
  bundle.storage.modules[id] = {};
  bundle.environment.modules[id] = {};
  const emitted = [];
  const context = {getModuleIds: () => [id][Symbol.iterator](),
    error: message => {throw new Error(message);}, emitFile: asset => emitted.push(asset)};
  assert.throws(() => plugin.generateBundle.call(context, {}, bundle), /runtime_sdk_owner_identity_violation/u);
  assert.equal(emitted.length, 0);
  delete bundle.environment.modules[id];
  plugin.generateBundle.call(context, {}, bundle);
  assert.deepEqual(JSON.parse(emitted[0].source).owners, [{module: "apps/web/src/core/runtime/appStorage.ts", chunks: ["sdk/v1/storage.js"]}]);
});

test("AI assistance and concept retrieval add only their finite original public entrypoints", async () => {
  const loaded = await load("1", "none");
  const entries = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk").api.facades;
  for (const [name, source, exports] of [
    ["ai-assistance", "pluginAiAssistance", ["getAiAssistanceGateway", "aiAssistanceErrorMessage"]],
    ["concept-retrieval", "conceptRetrieval", ["PaperConceptRetrievalControl"]],
  ]) assert.deepEqual(entries.find(entry => entry.name === name), {
    name, source: "apps/web/src/core/public/" + source + ".ts", exports,
  });
});

test("the document and HBS increment binds six exact Core leaves and two existing public package instances", async () => {
  const loaded = await load("1", "none");
  const plugins = loaded.config.plugins.flat(Infinity);
  const runtime = plugins.find(value => value?.name === "workbench-runtime-sdk");
  const source = plugins.find(value => value?.name === "workbench-sdk-source-bindings");
  const groups = [
    ["annotation-notes", "annotationNotes", ["listBrowserNotePageIds", "ensureBrowserAnnotationNote"]],
    ["documents", "documents", ["getNoteDocumentGateway"]],
    ["document-recovery", "documentRecovery", ["clearRecoveryDraft", "createRecoveryDraft", "loadRecoveryDraft", "saveRecoveryDraft"]],
    ["hbs", "hbs", ["HbsStructuredEditor", "directTextDiff", "getHbsDocumentGateway", "getPendingHbsDocumentCommit"]],
    ["literature-notes", "literatureNotes", ["stageLegacyAnnotationDocument", "requestLiteratureNoteTrash"]],
    ["structured-resources", "structuredResources", ["getStructuredResourceGateway", "structuredWriteValidationDiagnostic"]],
  ];
  assert.equal(groups.reduce((count, group) => count + group[2].length, 0), 15);
  for (const [name, module, exports] of groups) assert.deepEqual(runtime.api.facades.find(value => value.name === name), {
    name, source: "apps/web/src/core/public/" + module + ".ts", exports: [...exports, ...(writingExistingExtensions[name] ?? [])],
  });
  const pure = [
    {name: "core-hbs", source: "@research-assistant/core/hbs", exports: ["createStructuredDocumentAdapter", "DocumentVersionConflict", "HbsSession"]},
    {name: "data-hbs", source: "@research-assistant/data/hbs", exports: ["createDocumentPort", "createDocumentPreservationPort"]},
  ];
  for (const facade of pure) {
    assert.deepEqual(runtime.api.facades.find(value => value.name === facade.name), facade);
    assert.equal(runtime.load("\0virtual:workbench-runtime-sdk/" + facade.name),
      `export {${facade.exports.join(",")}} from ${JSON.stringify(facade.source)};`);
    assert.equal(source.resolveId(facade.source), undefined, "static public-package imports retain their normal resolution");
    assert.equal(source.resolveId("@research-assistant/workbench-sdk/" + facade.name), undefined,
      "bare packages are not invented SDK subpaths");
  }
  const sdk = JSON.parse(readFileSync(path.join(root, "ra-workbench-sdk/packages/workbench-sdk/package.json"), "utf8"));
  assert.equal(sdk.exports["./core-hbs"], undefined);
  assert.equal(sdk.exports["./data-hbs"], undefined);
});

test("the existing emitted-owner guard rejects duplicate chunks for every actual public HBS source", async () => {
  const loaded = await load("1", "none");
  const plugin = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk");
  const sourceNames = ["ra-workbench-sdk/packages/core/src/hbs/session.ts", "ra-workbench-sdk/packages/core/src/hbs/document.ts", "ra-workbench-sdk/packages/core/src/hbs/adapters.ts",
    "ra-workbench-sdk/packages/data/src/hbs/documentPort.ts", "ra-workbench-sdk/packages/data/src/hbs/documentPreservationPort.ts",
    "ra-workbench-sdk/packages/core/src/agent/promptTemplate.ts", "ra-workbench-sdk/packages/core/src/nal/queryCollections.ts", "ra-workbench-sdk/packages/core/src/reader/history.ts",
    "ra-workbench-sdk/packages/data/src/agent/conversationController.ts", "ra-workbench-sdk/packages/data/src/agent/reviewedSubmissionController.ts",
    "ra-workbench-sdk/packages/data/src/agent/reviewedSubmissionOwner.ts"];
  const sources = sourceNames.map(source => path.join(path.dirname(require.resolve("@research-assistant/" + source.split("/")[2])), source.split("/src/")[1].replace(/\.ts$/u, ".js")));
  const makeBundle = () => Object.fromEntries(plugin.api.facades.map(facade => [facade.name, {
    type: "chunk", fileName: "sdk/v1/" + facade.name + ".js", exports: facade.exports, imports: [], modules: {},
  }]));
  for (const source of sources) {
    const id = source, bundle = makeBundle();
    bundle["core-hbs"].modules[id] = {};
    bundle["data-hbs"].modules[id] = {};
    const context = {getModuleIds: () => [id][Symbol.iterator](),
      error: message => {throw new Error(message);}, emitFile: () => assert.fail("duplicate module cannot publish an identity receipt")};
    assert.throws(() => plugin.generateBundle.call(context, {}, bundle), /runtime_sdk_owner_identity_violation/u, source);
  }
  const bundle = makeBundle(), emitted = [];
  for (const source of sources) bundle["core-hbs"].modules[source] = {};
  plugin.generateBundle.call({getModuleIds: () => sources[Symbol.iterator](),
    error: message => {throw new Error(message);}, emitFile: asset => emitted.push(asset)}, {}, bundle);
  const receipt = JSON.parse(emitted[0].source);
  assert.deepEqual(receipt.owners, sources.sort().map(source => ({module: path.relative(path.join(root, "ra-workbench"), source), chunks: ["sdk/v1/core-hbs.js"]})));
});

test("actual canonical frontend imports resolve through the shared public-package facades", async () => {
  const loaded = await load("1", "none");
  const runtime = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk");
  const consumers = new Map();
  for (const plugin of ["literature_library", "writing_projects", "note_manager"]) {
    const sourceRoot = path.join(root, "plugins", {literature_library: "ra-literature-library", writing_projects: "ra-writing-projects", note_manager: "ra-note-manager"}[plugin], "src");
    const pending = [path.join(sourceRoot, "index.ts")], visited = new Set();
    while (pending.length) {
      const filename = pending.pop();
      if (visited.has(filename)) continue;
      visited.add(filename);
      const text = readFileSync(filename, "utf8");
      const source = ts.createSourceFile(filename, text, ts.ScriptTarget.Latest, true);
      for (const dependency of ts.preProcessFile(text).importedFiles) {
        if (!dependency.fileName.startsWith(".")) continue;
        const target = ts.resolveModuleName(dependency.fileName, filename, {moduleResolution: ts.ModuleResolutionKind.Bundler}, ts.sys).resolvedModule?.resolvedFileName;
        if (target?.startsWith(sourceRoot + path.sep) && /\.tsx?$/u.test(target)) pending.push(target);
      }
      for (const statement of source.statements) {
        if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
        const module = statement.moduleSpecifier.text, clause = statement.importClause;
        if (!/^@research-assistant\/(?:core|data|writing)\//u.test(module) || !clause || clause.isTypeOnly) continue;
        assert.ok(clause.namedBindings && ts.isNamedImports(clause.namedBindings), filename + ": named public values required");
        const values = consumers.get(module) ?? new Map();
        for (const element of clause.namedBindings.elements) {
          if (!element.isTypeOnly) values.set((element.propertyName ?? element.name).text, filename);
        }
        if (values.size) consumers.set(module, values);
      }
    }
  }
  assert.ok(consumers.size > 0, "canonical plugin consumer discovery must not be empty");
  for (const [module, values] of consumers) {
    const facade = runtime.api.facades.find(value => value.source === module);
    assert.ok(facade, module);
    for (const [value, filename] of values) assert.ok(facade.exports.includes(value), filename + ": " + module + ": " + value);
  }
});

test("literature shares the existing Ariakit and PDF.js values and Core worker asset", async () => {
  const loaded = await load("1", "none");
  const plugins = loaded.config.plugins.flat(Infinity);
  const runtime = plugins.find(value => value?.name === "workbench-runtime-sdk");
  const source = plugins.find(value => value?.name === "workbench-sdk-source-bindings");
  for (const facade of [
    {name: "ariakit-react", source: "@ariakit/react", exports: ["Tab", "TabList", "TabPanel", "TabProvider", "PopoverDisclosure", "PopoverProvider", "usePopoverStore", "useStoreState", "DialogHeading"]},
    {name: "pdfjs", source: "pdfjs-dist/legacy/build/pdf.mjs", exports: ["GlobalWorkerOptions", "getDocument"]},
    {name: "pdfjs-worker", source: "pdfjs-dist/legacy/build/pdf.worker.mjs?url", exports: ["default"]},
  ]) {
    assert.deepEqual(runtime.api.facades.find(value => value.name === facade.name), facade);
    assert.equal(runtime.load("\0virtual:workbench-runtime-sdk/" + facade.name), `export {${facade.exports.join(",")}} from ${JSON.stringify(facade.source)};`);
    assert.equal(source.resolveId(facade.source), undefined, "static imports retain their original package instance");
  }
});

test("capture, Docling and dictionary expose only their original finite ports", async () => {
  const loaded = await load("1", "none");
  const entries = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk").api.facades;
  for (const [name, source, exports] of [
    ["library-capture", "libraryCapture", ["listenBrowserCaptures", "subscribeWindowFileDrop", "desktopDropClientPosition"]],
    ["docling", "docling", ["getDoclingParserGateway", "doclingErrorMessage", "runDoclingParse"]],
    ["dictionary", "dictionary", ["getWordExplanationGateway", "createDictionaryGateway", "DictionaryLookup", "WordContextExplanation", "getWordExplanationStore"]],
  ]) assert.deepEqual(entries.find(entry => entry.name === name), {
    name, source: "apps/web/src/core/public/" + source + ".ts", exports,
  });
});

test("the remaining presentation and PDF leaves bind only their finite original values", async () => {
  const loaded = await load("1", "none");
  const entries = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-sdk").api.facades;
  const groups = [
    {name: "ui", source: "ui", exports: ["ContentTrustIndicator", "ContentTrustGroupControls", "CitationStyleSelect", "MouseLeftShortcut"]},
    {name: "structured-field-colors", source: "structuredFieldColors", exports: ["READING_FIELD_PALETTE", "readingFieldColor"]},
    {name: "text-documents", source: "textEditor", exports: ["openTextDocument"]},
    {name: "model-route-picker", source: "modelRoutePicker", exports: ["LlmRoutePicker"]},
    {name: "pdf", source: "pdf", exports: ["PdfViewController", "PDF_BACKGROUND_SUSPEND_MS", "PdfPageControls", "usePdfThumbnailCacheWindow", "PdfView", "PdfViewAdapter"]},
    {name: "pdf-geometry", source: "pdfGeometry", exports: ["positionPdfSelectionPopover", "clampPdfPageIndex", "parsePdfPageInput"]},
  ];
  for (const group of groups) assert.deepEqual(entries.find(entry => entry.name === group.name), {
    name: group.name, source: "apps/web/src/core/public/" + group.source + ".ts", exports: [...group.exports, ...(writingExistingExtensions[group.name] ?? [])],
  });
  const settings = entries.find(entry => entry.name === "settings");
  assert.deepEqual(settings.exports.slice(0, 3), ["SettingRow", "SettingGroup", "useDeveloperMode"], "the earlier three settings values retain their order");
  assert.deepEqual(settings.exports.slice(3), ["SystemDocumentDialog"], "this batch adds one original settings value");
});

test("the three packed SDK leaves match the actual Core public signatures in both directions", () => {
  const directory = mkdtempSync(path.join(tmpdir(), "runtime-sdk-leaf-contract-"));
  temporary.push(directory);
  const installed = path.join(directory, "node_modules/@research-assistant/workbench-sdk");
  mkdirSync(installed, {recursive: true});
  const archive = path.join(directory, "sdk.tgz");
  copySdkArtifact("workbench-sdk", archive);
  execFileSync("tar", ["-xzf", archive, "--strip-components=1", "-C", installed]);
  const coreInstalled = path.join(directory, "node_modules/@research-assistant/core");
  mkdirSync(coreInstalled, {recursive: true});
  const coreArchive = path.join(directory, "core.tgz");
  copySdkArtifact("core", coreArchive);
  execFileSync("tar", ["-xzf", coreArchive, "--strip-components=1", "-C", coreInstalled]);
  mkdirSync(path.join(directory, "node_modules/@types"), {recursive: true});
  symlinkSync(path.dirname(require.resolve("@types/react/package.json")), path.join(directory, "node_modules/@types/react"), "dir");
  const groups = [
    ["environment", "pluginRuntimeEnvironment", ["isAppPreview", "isDesktopRuntime", "getRuntimeEnvironment", "ensureRuntimeStructuredResources", "hasPlatformCapabilities"]],
    ["shortcuts", "settings", ["formatShortcut", "isShortcutMac"]],
    ["settings", "settings", ["SettingRow", "SettingGroup", "useDeveloperMode"]],
  ];
  const lines = [];
  for (const [sdk, core, names] of groups) {
    lines.push(`import type * as sdk_${sdk} from "@research-assistant/workbench-sdk/${sdk}";`);
    lines.push(`import type * as core_${sdk} from ${JSON.stringify(path.join(root, "ra-workbench/apps/web/src/core/public", core + ".ts"))};`);
    for (const name of names) {
      lines.push(`declare const core_${name}: typeof core_${sdk}.${name};`);
      lines.push(`declare const sdk_${name}: typeof sdk_${sdk}.${name};`);
      lines.push(`const acceptsSdk_${name}: typeof core_${sdk}.${name} = sdk_${name}; void acceptsSdk_${name};`);
      lines.push(`const acceptsCore_${name}: typeof sdk_${sdk}.${name} = core_${name}; void acceptsCore_${name};`);
    }
  }
  const source = path.join(directory, "consumer.ts");
  writeFileSync(source, lines.join("\n"));
  const config = path.join(directory, "tsconfig.json");
  writeFileSync(config, JSON.stringify({extends: path.join(root, "ra-workbench/tests/sdk-contracts/tsconfig.core.json"), compilerOptions: {
    noEmit: true, jsx: "react-jsx", lib: ["ES2022", "DOM", "DOM.Iterable"], allowImportingTsExtensions: true,
  }, files: [source, path.join(root, "ra-workbench/apps/web/src/env.d.ts")]}));
  const result = spawnSync(process.execPath, [require.resolve("typescript/bin/tsc"), "-p", config, "--pretty", "false"], {cwd: root, encoding: "utf8"});
  assert.equal(result.status, 0, result.stdout + result.stderr);
});

test("the six SDK declarations preserve the real Core public signatures in both directions", () => {
  const directory = mkdtempSync(path.join(tmpdir(), "runtime-sdk-contract-"));
  temporary.push(directory);
  const groups = [
    ["feedback", "feedback", ["useNotificationState"]],
    ["noteResources", "noteResources", ["useNoteAnnotations", "useLiteratureNotes"]],
    ["overlay", "overlay", ["OverlayPortal", "Popover", "Dialog", "useOverlayTarget"]],
    ["platform", "platform", ["open"]],
    ["reorder", "reorder", ["ReorderableListRow", "ReorderDragHandle"]],
    ["workbench", "workbench", ["usePublishWorkspaceTabs"]],
  ];
  const lines = [];
  for (const [sdk, core, names] of groups) {
    lines.push(`import type * as sdk_${sdk} from ${JSON.stringify(path.join(path.dirname(require.resolve("@research-assistant/workbench-sdk")), sdk + ".js"))};`);
    lines.push(`import type * as core_${sdk} from ${JSON.stringify(path.join(root, "ra-workbench/apps/web/src/core/public", core + ".ts"))};`);
    for (const name of names) {
      lines.push(`declare const core_${name}: typeof core_${sdk}.${name};`);
      lines.push(`declare const sdk_${name}: typeof sdk_${sdk}.${name};`);
      lines.push(`const acceptsSdk_${name}: typeof core_${sdk}.${name} = sdk_${name}; void acceptsSdk_${name};`);
      lines.push(`const acceptsCore_${name}: typeof sdk_${sdk}.${name} = core_${name}; void acceptsCore_${name};`);
    }
  }
  const source = path.join(directory, "consumer.ts");
  writeFileSync(source, lines.join("\n"));
  const config = path.join(directory, "tsconfig.json");
  writeFileSync(config, JSON.stringify({extends: path.join(root, "ra-workbench/tests/sdk-contracts/tsconfig.core.json"), compilerOptions: {
    noEmit: true, jsx: "react-jsx", lib: ["ES2022", "DOM", "DOM.Iterable"], allowImportingTsExtensions: true,
  }, files: [source, path.join(root, "ra-workbench/apps/web/src/env.d.ts")]}));
  const result = spawnSync(process.execPath, [require.resolve("typescript/bin/tsc"), "-p", config, "--pretty", "false"], {cwd: root, encoding: "utf8"});
  assert.equal(result.status, 0, result.stdout + result.stderr);
});

test("static SDK bindings reuse the exact runtime facade exports and Core module identities", async () => {
  const loaded = await load("1", "none");
  const plugins = loaded.config.plugins.flat(Infinity);
  const source = plugins.find(value => value?.name === "workbench-sdk-source-bindings");
  const runtime = plugins.find(value => value?.name === "workbench-runtime-sdk");
  assert.equal(source.enforce, "pre", "SDK bindings must run before Vite resolves declaration-only package files");
  assert.ok(source.api.facades.length > 0);
  for (const facade of source.api.facades) {
    assert.deepEqual(facade, runtime.api.facades.find(value => value.name === facade.name));
    const bound = source.resolveId("@research-assistant/workbench-sdk/" + facade.name);
    assert.equal(source.load(bound), runtime.load("\0virtual:workbench-runtime-sdk/" + facade.name));
    assert.ok(facade.source.startsWith("apps/web/src/core/public/"));
  }
  for (const id of ["@research-assistant/workbench-sdk/private-store", "@research-assistant/workbench-sdk/composition", "@research-assistant/workbench-sdk", "react"])
    assert.equal(source.resolveId(id), undefined);
});

test("the nineteen writing SDK leaves bind the same exact Core producers and declared runtime values", async () => {
  const loaded = await load("1", "none");
  const plugins = loaded.config.plugins.flat(Infinity);
  const runtime = plugins.find(value => value?.name === "workbench-runtime-sdk");
  const source = plugins.find(value => value?.name === "workbench-sdk-source-bindings");
  const producers = [
    ["block-content", "blockContent"], ["document-windows", "documentWindows"], ["editor-review", "editorReview"],
    ["code-editor-extensions", "codeEditorExtensions"], ["writing-storage", "writingStorage"], ["writing-runtime", "writingRuntime"],
    ["git-versions", "gitVersions"], ["git-worktrees", "gitWorktrees"], ["text-diff-stats", "textDiffStats"],
    ["agent-gateways", "agent"], ["agent-delivery", "pluginAgentDelivery"], ["agent-diagnostics", "pluginAgentDiagnostics"],
    ["agent-drafts", "pluginAgentDrafts"], ["agent-writing", "agentWriting"], ["agent-conversations", "pluginAgentConversations"],
    ["agent-context-controls", "pluginAgentContextControls"], ["agent-submission", "pluginAgentSubmission"],
    ["agent-presentation", "pluginAgentPresentation"], ["agent-canvas", "pluginAgentCanvas"],
  ];
  assert.deepEqual(producers.map(([name]) => name), writingCoreFacadeNames);
  const sdkRoot = path.join(root, "ra-workbench-sdk/packages/workbench-sdk");
  const sdk = JSON.parse(readFileSync(path.join(sdkRoot, "package.json"), "utf8"));
  const declarationFiles = producers.map(([name]) => path.join(sdkRoot, sdk.exports["./" + name].types));
  const program = ts.createProgram(declarationFiles, {module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler, skipLibCheck: true, strict: true, noEmit: true});
  const checker = program.getTypeChecker();
  for (const [index, [name, producer]] of producers.entries()) {
    const facade = runtime.api.facades.find(value => value.name === name);
    assert.equal(facade.source, "apps/web/src/core/public/" + producer + ".ts");
    const module = program.getSourceFile(declarationFiles[index]);
    assert.ok(module, name);
    const values = checker.getExportsOfModule(checker.getSymbolAtLocation(module)).filter(symbol => {
      const target = symbol.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol;
      return target.flags & ts.SymbolFlags.Value;
    }).map(symbol => symbol.name).sort();
    assert.deepEqual([...facade.exports].sort(), values, name + " must expose exactly its independently built declarations");
    assert.equal(source.load(source.resolveId("@research-assistant/workbench-sdk/" + name)),
      runtime.load("\0virtual:workbench-runtime-sdk/" + name));
  }
});

test("writing shares exactly the nine additional public and editor package facades", async () => {
  const loaded = await load("1", "none");
  const plugins = loaded.config.plugins.flat(Infinity);
  const runtime = plugins.find(value => value?.name === "workbench-runtime-sdk");
  const source = plugins.find(value => value?.name === "workbench-sdk-source-bindings");
  const additions = [
    {name: "core-document-review", source: "@research-assistant/core/document-review", exports: ["sameReviewDocument", "persistReviewSnapshots"]},
    {name: "codemirror-state", source: "@codemirror/state", exports: ["ChangeSet", "Compartment", "EditorSelection", "EditorState", "Facet", "Prec", "RangeSet", "RangeSetBuilder", "StateEffect", "StateField", "Text", "Transaction", "combineConfig"]},
    {name: "codemirror-view", source: "@codemirror/view", exports: ["Decoration", "EditorView", "GutterMarker", "ViewPlugin", "WidgetType", "activateHover", "drawSelection", "dropCursor", "getPanel", "gutter", "highlightActiveLine", "highlightActiveLineGutter", "hoverTooltip", "keymap", "lineNumbers", "logException", "rectangularSelection", "showPanel", "showTooltip"]},
    {name: "codemirror-merge", source: "@codemirror/merge", exports: ["diff", "getChunks", "getOriginalDoc", "originalDocChangeEffect", "unifiedMergeView"]},
    {name: "codemirror-commands", source: "@codemirror/commands", exports: ["copyLineDown", "defaultKeymap", "deleteLine", "history", "historyKeymap", "indentLess", "indentMore", "invertedEffects", "isolateHistory", "redo", "undo"]},
    {name: "codemirror-search", source: "@codemirror/search", exports: ["gotoLine", "highlightSelectionMatches", "openSearchPanel", "search", "searchKeymap"]},
    {name: "codemirror-language", source: "@codemirror/language", exports: ["StreamLanguage", "bracketMatching", "defaultHighlightStyle", "foldAll", "foldGutter", "foldKeymap", "indentOnInput", "indentUnit", "syntaxHighlighting", "toggleFold", "unfoldAll"]},
    {name: "codemirror-autocomplete", source: "@codemirror/autocomplete", exports: ["acceptCompletion", "clearSnippet", "closeBrackets", "closeBracketsKeymap", "closeCompletion", "completionKeymap", "insertCompletionText", "nextSnippetField", "pickedCompletion", "prevSnippetField", "snippet", "snippetKeymap", "startCompletion"]},
    {name: "blocknote-react", source: "@blocknote/react", exports: ["createReactBlockSpec", "useCreateBlockNote"]},
  ];
  assert.equal(runtime.api.facades.length, 93);
  assert.deepEqual(runtime.api.facades.filter(value => additions.some(addition => addition.name === value.name)), additions);
  const sdk = JSON.parse(readFileSync(path.join(root, "ra-workbench-sdk/packages/workbench-sdk/package.json"), "utf8"));
  for (const facade of additions) {
    assert.equal(runtime.load("\0virtual:workbench-runtime-sdk/" + facade.name),
      `export {${facade.exports.join(",")}} from ${JSON.stringify(facade.source)};`);
    assert.equal(source.resolveId(facade.source), undefined, "static consumers retain the original public package identity");
    assert.equal(source.resolveId("@research-assistant/workbench-sdk/" + facade.name), undefined);
    assert.equal(sdk.exports["./" + facade.name], undefined, "a bare package is not a synthetic SDK subpath");
  }
});

test("the runtime Core build rejects embedded canonical plugin implementations", async () => {
  const loaded = await load(undefined, "none");
  const plugin = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-core");
  assert.equal(typeof plugin.generateBundle, "function");
  plugin.configResolved({command: "build"});
  for (const source of ["plugins/ra-note-manager/src/features/private.ts", "plugins/ra-literature-library/src/components/private.ts"]) {
    assert.throws(() => plugin.generateBundle.call({
      getModuleIds: () => [path.join(root, source)][Symbol.iterator](),
      error: message => {throw new Error(message);}, emitFile: () => undefined,
    }), /plugin_build_closure_violation/u);
  }
});

test("the Core closure guard emits its empty plugin receipt", async () => {
  const loaded = await load(undefined, "none");
  const plugin = loaded.config.plugins.flat(Infinity).find(value => value?.name === "workbench-runtime-core");
  plugin.configResolved({command: "build"});
  const emitted = [];
  plugin.generateBundle.call({
    getModuleIds: () => [path.join(root, "ra-workbench/apps/web/src/core/public/layout.ts")][Symbol.iterator](),
    error: message => {throw new Error(message);}, emitFile: asset => emitted.push(asset),
  });
  const receipt = JSON.parse(emitted[0].source);
  assert.deepEqual(receipt.plugins, []);
  assert.deepEqual(receipt.modules, ["apps/web/src/core/public/layout.ts"]);
});

test("the actual SDK consumer checker rejects root plugin modules as well as old application modules", () => {
  const directory = mkdtempSync(path.join(tmpdir(), "runtime-sdk-plugin-guard-"));
  temporary.push(directory);
  const core = path.join(directory, "core"), plugin = path.join(directory, "plugin");
  mkdirSync(path.join(core, "sdk/v1"), {recursive: true}); mkdirSync(plugin);
  writeFileSync(path.join(core, "workbench-composition.json"), JSON.stringify({plugins: []}));
  writeFileSync(path.join(core, "sdk/v1/example.js"), "export const value = 1;");
  writeFileSync(path.join(plugin, "entry.js"), 'import {value} from "/sdk/v1/example.js"; console.log(value);');
  const graph = {modules: [], facadeBuild: "same-core-multi-entry", owners: [
    {module: "apps/web/src/core/features/notes/NotesClient.tsx", chunks: ["notes.js"]},
    {module: "node_modules/react/index.js", chunks: ["react.js"]},
  ], facades: [{fileName: "sdk/v1/example.js", exports: ["value"]}]};
  for (const source of ["apps/web/src/plugins/note_manager/private.ts", "plugins/ra-note-manager/src/privateTypes.ts", "C:\\repo\\plugins\\note_manager\\src\\private.ts"]) {
    writeFileSync(path.join(core, "runtime-sdk.json"), JSON.stringify({...graph, modules: [source]}));
    const rejected = spawnSync(process.execPath, ["ra-workbench/scripts/check-runtime-sdk-build.mjs", core, plugin], {cwd: root, encoding: "utf8"});
    assert.equal(rejected.status, 1);
    assert.match(rejected.stderr, /must exclude application and root plugin implementations/u);
  }
  writeFileSync(path.join(core, "runtime-sdk.json"), JSON.stringify({...graph, modules: ["apps/web/src/core/feature.ts"]}));
  const allowed = spawnSync(process.execPath, ["ra-workbench/scripts/check-runtime-sdk-build.mjs", core, plugin], {cwd: root, encoding: "utf8"});
  assert.equal(allowed.status, 0, allowed.stderr);
});
