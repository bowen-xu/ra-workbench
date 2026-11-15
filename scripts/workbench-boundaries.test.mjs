import { afterEach, describe, expect, it } from "vitest";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { boundaryKey, collectWorkbenchImports, checkWorkbenchBoundaries, findExcludedWorkbenchPluginSources } from "./workbench-boundaries.mjs";

const roots = [];
function fixture(files) {
  const root = mkdtempSync(join(tmpdir(), "workbench-boundaries-"));
  roots.push(root);
  for (const [file, text] of Object.entries(files)) {
    const target = join(root, file);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, text);
  }
  return root;
}
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });

const core = "apps/web/src/core/", plugin = "apps/web/src/plugins/demo/";
const policy = {
  schemaVersion: 1,
  publicEntries: [core + "public/ui.ts", plugin + "public/workspace.ts"],
  contracts: [core + "contracts/ui.ts", plugin + "public/contracts.ts"],
};
const empty = { schemaVersion: 1, workItems: { migration: "Migrate exact old edges" }, exceptions: [] };
const check = root => checkWorkbenchBoundaries(collectWorkbenchImports(root), policy, empty);
const support = {
  [core + "private.ts"]: "export const value = 1; export const other = 2;",
  [core + "public/ui.ts"]: 'export {value} from "../private";',
  [core + "contracts/ui.ts"]: "export interface View {id: string}",
  [plugin + "private.ts"]: "export const pluginValue = 1;",
  [plugin + "public/workspace.ts"]: 'export {pluginValue} from "../private";',
};
const runtimeSource = core + "runtime/plugins/loader.ts";
const runtimePort = {source: runtimeSource, kind: "dynamic-import", expression: "entryUrl",
  function: "importRuntimeModule", count: 1, description: "Reviewed Core package ESM port; not a permission sandbox."};
const runtimePolicy = {...policy, runtimeImportPorts: [runtimePort]};

describe("Web ownership boundaries", () => {
  it("rejects absent plugin implementation and private types in both source layouts", () => {
    const sources = [core + "public/ui.ts", plugin + "private.ts", plugin + "public/contracts/ui.ts",
      "plugins/demo/src/features/privateTypes.ts", "plugins/demo/src/public/contracts.ts"];
    const pureContract = "plugins/demo/src/public/contracts.ts";
    expect(findExcludedWorkbenchPluginSources(sources, [], [pureContract])).toEqual(sources.slice(1, -1));
    expect(findExcludedWorkbenchPluginSources(sources, ["demo"], [pureContract])).toEqual([]);
    expect(findExcludedWorkbenchPluginSources([pureContract], [])).toEqual([pureContract]);
  });

  it("scans root plugin src including tests and nested lib, without scanning package build output", () => {
    const root = fixture({...support,
      "plugins/demo/src/feature.test.ts": 'import {value} from "/src/core/private.ts";',
      "plugins/demo/src/lib/feature.ts": 'import {value} from "/src/core/private.ts";',
      "plugins/demo/lib/generated.ts": 'import {value} from "/src/core/private.ts";',
      "plugins/demo/node_modules/foreign/src/feature.ts": 'import {value} from "/src/core/private.ts";',
    });
    expect(check(root).violations.map(edge => edge.from)).toEqual([
      "plugins/demo/src/feature.test.ts", "plugins/demo/src/lib/feature.ts",
    ]);
  });

  it.each(["literature_library", "writing_projects"])("resolves the canonical %s dev URL without granting private test access", id => {
    const target = `plugins/${id}/src/features/private.ts`;
    const root = fixture({...support,
      [target]: "export const value = 1;",
      "tests/ui/writing.pw.ts": `const value = import("/plugins/${id}/src/features/private.ts");`,
    });
    const report = check(root);
    expect(report.violations).toEqual([expect.objectContaining({
      from: "tests/ui/writing.pw.ts", target, kind: "dynamic-import", names: "*", reason: "consumer-private-import",
    })]);
    const baseline = {...empty, exceptions: [["tests/ui/writing.pw.ts", target, "dynamic-import", "*", 1, "migration"]]};
    expect(checkWorkbenchBoundaries(collectWorkbenchImports(root), policy, baseline).ok).toBe(true);
  });

  it.each([
    ['import {value} from "/src/core/private.ts";', "plugin-private-import"],
    ['import type {value} from "/src/core/private.ts";', "plugin-private-import"],
    ['export {value} from "/src/core/private.ts";', "plugin-private-import"],
    ['new URL("/src/core/private.ts", import.meta.url);', "plugin-private-import"],
    ['import {adapter} from "/src/app/private.ts";', "application-reverse-import"],
    ['import {invoke} from "@tauri-apps/api/core";', "plugin-platform-import"],
    ['import {foreign} from "../../other/src/private";', "plugin-private-import"],
  ])("keeps a root plugin inside the original boundary: %s", (text, reason) => {
    const root = fixture({...support, "plugins/demo/src/feature.ts": text,
      "apps/web/src/app/private.ts": "export const adapter = true;",
      "plugins/other/src/private.ts": "export const foreign = true;"});
    expect(check(root).violations).toContainEqual(expect.objectContaining({from: "plugins/demo/src/feature.ts", reason}));
  });

  it("allows only exact root public entries and pure contracts across owners", () => {
    const entry = "plugins/demo/src/public/workspace.ts", contract = "plugins/demo/src/public/contracts.ts";
    const root = fixture({...support,
      [entry]: 'export {pluginValue} from "../private";',
      [contract]: 'export type {View} from "/src/core/contracts/ui.ts";',
      "plugins/demo/src/private.ts": "export const pluginValue = 1;",
      "apps/web/src/app/bootstrap.ts": `import {pluginValue} from ${JSON.stringify(join("../../../..", entry))};`,
    });
    const scoped = {...policy, publicEntries: [...policy.publicEntries, entry], contracts: [...policy.contracts, contract]};
    expect(checkWorkbenchBoundaries(collectWorkbenchImports(root), scoped, empty).ok).toBe(true);
    writeFileSync(join(root, contract), "export const runtime = 1;");
    expect(checkWorkbenchBoundaries(collectWorkbenchImports(root), scoped, empty).violations[0]).toMatchObject({reason: "contract-runtime-code"});
  });

  it("does not let Core import root implementations or private types", () => {
    for (const typeOnly of [false, true]) {
      const edge = {from: core + "feature.ts", target: "plugins/demo/src/private.ts", kind: "import", names: "pluginValue", typeOnly, line: 1};
      const baseline = {...empty, exceptions: [[edge.from, edge.target, edge.kind, edge.names, 1, "migration"]]};
      expect(checkWorkbenchBoundaries([edge], policy, baseline).violations[0]).toMatchObject({reason: "core-plugin-implementation"});
    }
    const contract = "plugins/demo/src/public/contracts.ts", scoped = {...policy, contracts: [...policy.contracts, contract]};
    expect(checkWorkbenchBoundaries([{from: core + "feature.ts", target: contract, kind: "import", names: "type:View", typeOnly: true, line: 1}], scoped, empty).ok).toBe(true);
  });

  it("keeps the exact virtual workbench entry App-owned rather than an unchecked external package", () => {
    const target = "apps/web/src/app/workbenchComposition.ts";
    const root = fixture({...support, [target]: "export const composition = {};",
      "apps/web/src/main.selected.tsx": 'import {composition} from "virtual:workbench-composition";'});
    expect(check(root).ok).toBe(true);
    for (const from of [core + "feature.ts", plugin + "feature.ts"]) {
      const denied = fixture({...support, [target]: "export const composition = {};", [from]: 'import {composition} from "virtual:workbench-composition";'});
      expect(check(denied).violations[0]).toMatchObject({reason: "application-reverse-import", target});
    }
  });
  it("admits only static test imports of an exact App-owned test assembly entry", () => {
    const entry = "apps/web/src/app/Configuration.testSupport.tsx", scoped = {...policy, testAssemblyEntries: [entry]};
    const root = fixture({...support, [entry]: "export const bind = true;",
      [plugin + "feature.test.tsx"]: 'import {bind} from "../../app/Configuration.testSupport";'});
    expect(checkWorkbenchBoundaries(collectWorkbenchImports(root), scoped, empty).ok).toBe(true);
    for (const from of [core + "feature.spec.ts", plugin + "feature.test.tsx", "tests/unit/feature.test.ts"])
      expect(checkWorkbenchBoundaries([{from, target: entry, kind: "import", names: "bind", line: 1}], scoped, empty).ok).toBe(true);
    for (const [from, kind, target] of [
      [core + "feature.ts", "import", entry], [plugin + "feature.tsx", "import", entry],
      ["apps/web/src/app/Production.tsx", "import", entry], ["apps/web/src/app/feature.spec.tsx", "dynamic-import", entry],
      [plugin + "feature.test.tsx", "dynamic-import", entry], [plugin + "feature.test.tsx", "mock", entry],
      [plugin + "feature.test.tsx", "export", entry], [plugin + "feature.test.tsx", "import", "apps/web/src/app/Private.testSupport.tsx"],
      ["tests/ui/feature.pw.ts", "import", entry], ["tests/unit/feature.test.js", "import", entry],
    ]) expect(checkWorkbenchBoundaries([{from, target, kind, names: "bind", line: 1}], scoped, empty).violations[0]).toMatchObject({reason: "application-reverse-import"});
  });
  it("rejects broad, duplicated or public test assembly entry declarations", () => {
    const entry = "apps/web/src/app/Configuration.testSupport.tsx";
    for (const path of ["apps/web/src/app/*.testSupport.tsx", "apps/web/src/app/?.testSupport.tsx", "apps/web/src/app/[x].testSupport.tsx", core + "Configuration.testSupport.tsx", "apps/web/src/app/../core/Configuration.testSupport.tsx", "apps/web/src/app/Configuration.tsx"])
      expect(() => checkWorkbenchBoundaries([], {...policy, testAssemblyEntries: [path]}, empty)).toThrow("exact application");
    expect(() => checkWorkbenchBoundaries([], {...policy, testAssemblyEntries: [entry, entry]}, empty)).toThrow("exact application");
    expect(() => checkWorkbenchBoundaries([], {...policy, publicEntries: [...policy.publicEntries, entry], testAssemblyEntries: [entry]}, empty)).toThrow("not a plugin API");
    expect(() => checkWorkbenchBoundaries([], {...policy, contracts: [...policy.contracts, entry], testAssemblyEntries: [entry]}, empty)).toThrow("not a plugin API");
  });
  it("allows a browser driver to launch only a listed application test entry", () => {
    const entry = "apps/web/src/app/notes/Continuity.browser.tsx";
    const root = fixture({...support, [entry]: "export const probe = true;",
      "tests/ui/continuity.pw.ts": 'void import("/src/app/notes/Continuity.browser.tsx");'});
    const scoped = {...policy, browserTestEntries: [entry]};
    expect(checkWorkbenchBoundaries(collectWorkbenchImports(root), scoped, empty).ok).toBe(true);
    expect(check(root).violations[0]).toMatchObject({reason: "application-reverse-import", target: entry});
    for (const [from, kind, target] of [
      [core + "feature.ts", "dynamic-import", entry],
      [plugin + "feature.ts", "dynamic-import", entry],
      [plugin + "feature.test.ts", "dynamic-import", entry],
      ["tests/unit/feature.test.ts", "dynamic-import", entry],
      ["tests/ui/continuity.pw.ts", "import", entry],
      ["tests/ui/continuity.pw.ts", "dynamic-import", "apps/web/src/app/notes/Private.browser.tsx"],
    ]) {
      const edge = {from, target, kind, names: "*", typeOnly: false, line: 1};
      expect(checkWorkbenchBoundaries([edge], scoped, empty).violations[0]).toMatchObject({reason: "application-reverse-import"});
    }
  });
  it("rejects broad or non-application browser test entry declarations", () => {
    for (const entry of ["apps/web/src/app/*.browser.tsx", core + "private.browser.tsx", "apps/web/src/app/../core/private.browser.tsx"])
      expect(() => checkWorkbenchBoundaries([], {...policy, browserTestEntries: [entry]}, empty)).toThrow("exact application");
  });
  it("limits shared browser fixture helpers to exact test consumers and dynamic entry imports", () => {
    const entry = "apps/web/src/app/previews/AgentPreview.browser.tsx";
    const helper = "tests/ui/live-agent.ts";
    const scoped = {...policy, browserTestEntries: [entry], browserTestHelpers: [helper]};
    const edge = {from: helper, target: entry, kind: "dynamic-import", names: "*", typeOnly: false, line: 1};
    expect(checkWorkbenchBoundaries([edge], scoped, empty).ok).toBe(true);
    expect(checkWorkbenchBoundaries([{...edge, kind: "import", typeOnly: true}], scoped, empty).ok).toBe(true);
    for (const forbidden of [
      {...edge, from: "tests/ui/other-helper.ts"},
      {...edge, from: "apps/web/src/app/bootstrap.ts"},
      {...edge, from: core + "feature.ts"},
      {...edge, from: plugin + "feature.ts"},
      {...edge, kind: "import"},
      {...edge, target: "apps/web/src/app/previews/PrivateFixture.ts"},
    ]) expect(checkWorkbenchBoundaries([forbidden], scoped, empty).violations[0])
      .toMatchObject({reason: "application-reverse-import"});
  });
  it("rejects wildcard, production and duplicate browser fixture helper declarations", () => {
    for (const helper of ["tests/ui/*.ts", "tests/ui/../secret.ts", "apps/web/src/app/helper.ts", "tests/unit/helper.ts"])
      expect(() => checkWorkbenchBoundaries([], {...policy, browserTestHelpers: [helper]}, empty))
        .toThrow("exact tests/ui helper files");
    expect(() => checkWorkbenchBoundaries([], {...policy, browserTestHelpers: ["tests/ui/a.ts", "tests/ui/a.ts"]}, empty))
      .toThrow("exact tests/ui helper files");
  });
  it("permits a Core-only test to reject plugin loading without importing the plugin", () => {
    const root=fixture({...support,[core+"shell.test.ts"]:'vi.mock("../plugins/demo/public/workspace",()=>{throw Error("plugin was loaded");});'});
    expect(check(root).ok).toBe(true);
    writeFileSync(join(root,core+"shell.test.ts"),'import {pluginValue} from "../plugins/demo/public/workspace";');
    expect(check(root).violations[0]).toMatchObject({reason:"core-plugin-implementation"});
  });
  it("lets application assembly choose Core adapters without granting that access to plugins", () => {
    const root=fixture({...support,"apps/web/src/app/bootstrap.ts":'import {value} from "../core/private";'});
    expect(check(root).ok).toBe(true);
    writeFileSync(join(root,plugin+"feature.ts"),'import {value} from "../../core/private";');
    expect(check(root).violations[0]).toMatchObject({reason:"plugin-private-import"});
  });
  it("does not grandfather Core implementation dependencies on application or plugin implementations", () => {
    for (const target of [plugin + "private.ts", "apps/web/src/app/adapter.ts"]) {
      const edge = {from:core + "feature.ts",target,kind:"import",names:"value",typeOnly:false,line:1};
      const baseline = {...empty,exceptions:[[edge.from,edge.target,edge.kind,edge.names,1,"migration"]]};
      expect(checkWorkbenchBoundaries([edge],policy,baseline).violations).toHaveLength(1);
    }
  });
  it("lets plugin entrypoints declare Core contracts and call explicit public capabilities", () => {
    const root = fixture({ ...support,
      [plugin + "public/workspace.ts"]: 'import type {View} from "../../../core/contracts/ui"; import {value} from "../../../core/public/ui"; export const pluginValue: View = {id:String(value)};',
    });
    expect(check(root).ok).toBe(true);
    writeFileSync(join(root, plugin + "public/workspace.ts"), 'export {value} from "../../../core/private";');
    expect(check(root).violations[0]).toMatchObject({reason:"public-cross-owner-implementation"});
  });
  it("allows static composition and explicit public calls while keeping implementation exports narrow", () => {
    const root = fixture({ ...support,
      "apps/web/src/app/App.ts": 'import {pluginValue} from "../plugins/demo/public/workspace"; import {value} from "../core/public/ui";',
      [plugin + "feature.ts"]: 'import {value} from "../../core/public/ui";',
      [plugin + "public/contracts.ts"]: 'import type {View} from "../../../core/contracts/ui"; export type PluginView = View;',
    });
    expect(check(root)).toMatchObject({ ok: true, legacyReferences: 0 });
  });

  it.each([
    ['import {value} from "../../core/private";', "import"],
    ['import type {value} from "../../core/private";', "import"],
    ['export {value} from "../../core/private";', "export"],
    ['export type X = import("../../core/private").value;', "import-type"],
    ['void import("../../core/private");', "dynamic-import"],
    ['require("../../core/private");', "require"],
    ['vi.mock("../../core/private", () => ({}));', "mock"],
  ])("rejects a new private dependency: %s", (source, kind) => {
    const root = fixture({ ...support, [plugin + "feature.ts"]: source });
    expect(check(root).violations).toContainEqual(expect.objectContaining({ target: core + "private.ts", kind, reason: "plugin-private-import" }));
  });

  it.each(['/src/core/private.ts', '/src/core/private.ts?raw'])("checks Vite source URLs: %s", path => {
    const root = fixture({ ...support, [plugin + "feature.ts"]: `void import("${path}");` });
    expect(check(root).violations[0]).toMatchObject({target: core + "private.ts", reason: "plugin-private-import"});
  });

  it("checks worker/resource module URLs", () => {
    const root = fixture({ ...support, [plugin + "feature.ts"]: 'new URL("../../core/private.ts", import.meta.url);' });
    expect(check(root).violations[0]).toMatchObject({target: core + "private.ts", kind: "resource-url"});
  });

  it("keeps generated runtime resources Core-private and resolves their real URL target", () => {
    const root = fixture({...support, "apps/web/src/.generated/runtime.js": "export const x = 1;",
      "apps/web/src/.generated/runtime.d.ts": "export declare const x: number;",
      [core + "feature.ts"]: 'new URL("../.generated/runtime.js", import.meta.url);'});
    expect(check(root).ok).toBe(true);
    writeFileSync(join(root, plugin + "feature.ts"), 'new URL("../../.generated/runtime.js", import.meta.url);');
    expect(check(root).violations[0]).toMatchObject({target: "apps/web/src/.generated/runtime.js", reason: "plugin-private-import"});
  });

  it("checks tsconfig aliases against the resolved private file, not the spelling of an import", () => {
    const root = fixture({ ...support,
      "apps/web/tsconfig.app.json": JSON.stringify({ compilerOptions: { baseUrl: ".", paths: { "@hidden": ["src/core/private.ts"] } } }),
      [plugin + "feature.ts"]: 'import {value} from "@hidden";',
    });
    expect(check(root).violations[0]).toMatchObject({ target: core + "private.ts", reason: "plugin-private-import" });
  });

  it("does not treat a contract alias spelled like a shared package as a public type", () => {
    const root = fixture({...support,
      "apps/web/tsconfig.app.json": JSON.stringify({compilerOptions:{baseUrl:".",paths:{"@research-assistant/fake":["src/core/private.ts"]}}}),
      [core + "contracts/ui.ts"]: 'import type {value} from "@research-assistant/fake"; export type X = typeof value;',
    });
    expect(check(root).violations[0]).toMatchObject({reason:"contract-private-import"});
  });

  it("rejects reverse core dependencies even when the plugin offers a public implementation", () => {
    const root = fixture({ ...support, [core + "feature.ts"]: 'import {pluginValue} from "../plugins/demo/public/workspace";' });
    expect(check(root).violations[0]).toMatchObject({ reason: "core-plugin-implementation" });
  });

  it("does not exclude preview/test files or computed import expressions", () => {
    const root = fixture({ ...support, [plugin + "Preview.test.ts"]: "const name = './x'; void import(name);" });
    expect(check(root).violations[0]).toMatchObject({ reason: "computed-import", names: "name" });
  });

  it("allows only the reviewed Core runtime import site without a migration exception", () => {
    const root = fixture({...support, [runtimeSource]: 'async function importRuntimeModule(entryUrl: string) { return import(/* @vite-ignore */ entryUrl); }'});
    const edges = collectWorkbenchImports(root);
    expect(edges.find(edge => edge.computed)).toMatchObject({from: runtimeSource, kind: "dynamic-import", names: "entryUrl", enclosingFunction: "importRuntimeModule"});
    expect(checkWorkbenchBoundaries(edges, runtimePolicy, empty)).toMatchObject({ok: true, legacyReferences: 0});
    expect(checkWorkbenchBoundaries(edges, policy, empty).violations[0]).toMatchObject({reason: "computed-import"});
  });

  it.each([
    [runtimeSource, 'async function other(entryUrl: string) { return import(entryUrl); }'],
    [core + "runtime/plugins/other.ts", 'async function importRuntimeModule(entryUrl: string) { return import(entryUrl); }'],
    [runtimeSource, 'async function importRuntimeModule(entryUrl: string) { return import(entryUrl + "?other"); }'],
    [runtimeSource, 'async function importRuntimeModule(entryUrl: string) { return import(resolveEntry(entryUrl)); }'],
    [runtimeSource, 'async function importRuntimeModule(entryUrl: string) { return (() => import(entryUrl))(); }'],
    [runtimeSource, 'async function importRuntimeModule(entryUrl: string) { const nested = () => import(entryUrl); return nested(); }'],
    [runtimeSource, 'async function importRuntimeModule(entryUrl: string) { return require(entryUrl); }'],
    [runtimeSource, 'const entryUrl = "x"; void import(entryUrl);'],
  ])("rejects an unreviewed computed runtime site in %s: %s", (file, source) => {
    const root = fixture({...support, [file]: source});
    expect(checkWorkbenchBoundaries(collectWorkbenchImports(root), runtimePolicy, empty).violations).toContainEqual(expect.objectContaining({from: file, reason: "computed-import"}));
  });

  it("rejects a second runtime import occurrence even when a legacy budget would cover it", () => {
    const root = fixture({...support, [runtimeSource]: 'async function importRuntimeModule(entryUrl: string) { await import(entryUrl); return import(entryUrl); }'});
    const edges = collectWorkbenchImports(root), edge = edges.find(value => value.computed);
    const baseline = {...empty, exceptions: [[edge.from, edge.target, edge.kind, edge.names, 2, "migration"]]};
    expect(checkWorkbenchBoundaries(edges, runtimePolicy, empty).violations).toEqual([expect.objectContaining({reason: "computed-import", enclosingFunction: "importRuntimeModule"})]);
    expect(checkWorkbenchBoundaries(edges, runtimePolicy, baseline).violations).toHaveLength(1);
  });

  it("rejects broad, duplicate or widened permanent runtime import declarations", () => {
    for (const change of [
      {source: core + "runtime/plugins/*.ts"}, {source: "apps/web/src/app/loader.ts"},
      {source: core + "runtime/../plugins/loader.ts"}, {source: core + "loader.test.ts"},
      {kind: "require"}, {expression: "entryUrl + suffix"}, {function: "*"}, {count: 2}, {count: 0}, {description: ""},
    ]) expect(() => checkWorkbenchBoundaries([], {...policy, runtimeImportPorts: [{...runtimePort, ...change}]}, empty)).toThrow("exact Core");
    expect(() => checkWorkbenchBoundaries([], {...policy, runtimeImportPorts: [runtimePort, runtimePort]}, empty)).toThrow("Duplicate runtime import port");
  });

  it("never turns a contract or public entry into a runtime loading port", () => {
    for (const source of [core + "contracts/unlisted.ts", core + "runtime/contracts.ts", core + "public/unlisted.ts"]) {
      expect(() => checkWorkbenchBoundaries([], {...policy, runtimeImportPorts: [{...runtimePort, source}]}, empty)).toThrow("cannot be a public entry or contract");
    }
    for (const key of ["contracts", "publicEntries"]) {
      expect(() => checkWorkbenchBoundaries([], {...runtimePolicy, [key]: [...policy[key], runtimeSource]}, empty)).toThrow("cannot be a public entry or contract");
    }
    const root = fixture({...support, [core + "contracts/ui.ts"]: 'export async function importRuntimeModule(entryUrl: string) { return import(entryUrl); }'});
    expect(checkWorkbenchBoundaries(collectWorkbenchImports(root), runtimePolicy, empty).violations.map(value => value.reason))
      .toEqual(expect.arrayContaining(["contract-runtime-code", "contract-runtime-import"]));
  });

  it.each(['@import "../../core/private.css";', '@import url(../../core/private.css);'])("checks cross-owner stylesheet imports: %s", source => {
    const root = fixture({ [core + "private.css"]: "a {}", [plugin + "view.css"]: source });
    expect(check(root).violations[0]).toMatchObject({ kind: "css-import", reason: "plugin-private-import" });
  });

  it("forbids private types, runtime dependencies and star exports in new public surfaces", () => {
    const root = fixture({ ...support,
      [plugin + "public/contracts.ts"]: 'import type {value} from "../../../core/private"; import "react";',
      [core + "public/ui.ts"]: 'export * from "../private";',
    });
    expect(check(root).violations.map(edge => edge.reason)).toEqual(expect.arrayContaining([
      "contract-private-import", "contract-runtime-import", "public-star-export",
    ]));
  });

  it("rejects executable code in a contract even without imports", () => {
    const root = fixture({...support, [plugin + "public/contracts.ts"]: "export function activate() { return 1; }"});
    expect(check(root).violations[0]).toMatchObject({reason: "contract-runtime-code"});
  });

  it("requires the exact imported names and occurrence count and reports retired exceptions", () => {
    const root = fixture({ ...support, [plugin + "feature.ts"]: 'import {value} from "../../core/private";' });
    const edges = collectWorkbenchImports(root), edge = edges.find(row => row.from === plugin + "feature.ts");
    const baseline = { ...empty, exceptions: [[edge.from, edge.target, edge.kind, edge.names, 1, "migration"]] };
    expect(checkWorkbenchBoundaries(edges, policy, baseline).ok).toBe(true);
    const differentName = { ...edge, names: "other" };
    expect(checkWorkbenchBoundaries([differentName], policy, baseline)).toMatchObject({ ok: false });
    expect(checkWorkbenchBoundaries([...edges, edge], policy, baseline).violations).toHaveLength(1);
    expect(checkWorkbenchBoundaries([], policy, baseline).stale).toEqual([expect.objectContaining({ key: boundaryKey(edge), current: 0 })]);
  });

  it("never uses a migration exception to legitimize a new public surface", () => {
    const edge = { from: core + "public/ui.ts", target: plugin + "private.ts", kind: "import", names: "pluginValue", typeOnly: false, line: 1 };
    const baseline = { ...empty, exceptions: [[edge.from, edge.target, edge.kind, edge.names, 1, "migration"]] };
    expect(checkWorkbenchBoundaries([edge], policy, baseline).violations).toHaveLength(1);
  });
});
