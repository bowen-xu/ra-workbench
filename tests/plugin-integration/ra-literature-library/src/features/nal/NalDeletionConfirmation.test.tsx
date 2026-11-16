// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {emptyNalState, type NalCollectionResult, type NalStatement} from "@research-assistant/core/nal";
import {notify} from "@research-assistant/workbench-sdk/feedback";
import {selectAppRuntime} from "../../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {NalManagementPanel} from "../../../../../../../plugins/ra-literature-library/src/features/nal/NalManagementPanel";
import {NalStatements} from "../../../../../../../plugins/ra-literature-library/src/features/nal/NalStatements";
import {NalQueryDirectory} from "../../../../../../../plugins/ra-literature-library/src/features/nal/NalQueryDirectory";
import type {NalController} from "../../../../../../../plugins/ra-literature-library/src/features/nal/useNal";

type Deletion = "concept" | "statement" | "collection";
const deletions: readonly Deletion[] = ["concept", "statement", "collection"];
const prompts: Record<Deletion, string> = {
  concept: "删除概念“概念甲”？它参与的文献关系、传递规则和笔记关联会移除；文献、笔记正文和来源历史保留。",
  statement: "删除语句“概念甲 → 概念乙”？文献、概念和笔记本身保留。",
  collection: "删除“查询甲”及其子查询？文献和关系会保留。",
};
const statement: NalStatement = {id: "statement-a", subject: {kind: "concept", id: "concept-a"},
  predicate: {kind: "concept", id: "concept-b"}, origin: "human"};
let host: HTMLDivElement, root: Root;
const originalConfirm = Object.getOwnPropertyDescriptor(window, "confirm");

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, fail) => {resolve = accept; reject = fail;});
  return {promise, resolve, reject};
}
function collection(id: string, name: string, parentId?: string): NalCollectionResult {
  return {definition: {id, name, conditions: [], ...(parentId ? {parentId} : {})},
    conditions: [], paperIds: [], matches: [], bindings: []};
}
function controller(): NalController {
  const concepts = [{id: "concept-a", label: "概念甲", explanation: "", revision: 1},
    {id: "concept-b", label: "概念乙", explanation: "", revision: 1}];
  return {
    state: {...emptyNalState(), revision: 7, concepts, statements: [statement]},
    ready: true, busy: false, query: undefined, outputVariables: undefined, setQuery: vi.fn(),
    refresh: vi.fn(async () => true), mutate: vi.fn(async () => true),
    vocabulary: concepts.map(c => ({atom: {kind: "concept", id: c.id}, label: c.label})),
    matches: [], allowed: new Set(), selectedCollectionId: undefined, selectCollection: vi.fn(),
    collections: [collection("query-a", "查询甲"), collection("query-child", "子查询", "query-a"), collection("query-b", "查询乙")],
    saveCollection: vi.fn(async () => true), removeCollection: vi.fn(async () => true),
    editCollections: vi.fn(async () => true), cacheError: undefined,
  };
}
function button(label: string): HTMLButtonElement {
  const element = [...document.querySelectorAll("button")].find(b => b.getAttribute("aria-label") === label);
  if (!element) throw new Error(`Missing production button: ${label}`);
  return element;
}
async function render(kind: Deletion, value: NalController) {
  await act(async () => root.render(kind === "concept"
    ? <NalManagementPanel controller={value} onClose={() => undefined} onAnnotate={() => undefined}/>
    : kind === "statement" ? <NalStatements controller={value} statements={value.state.statements} onAnnotate={() => undefined}/>
      : <NalQueryDirectory controller={value}/>));
}
async function removeButton(kind: Deletion): Promise<HTMLButtonElement> {
  if (kind === "concept") {
    await act(async () => button("概念甲 · concept-a").click());
    return button("删除概念：概念甲");
  }
  if (kind === "statement") return button("删除语句：概念甲 → 概念乙");
  await act(async () => button("查询操作：查询甲").click());
  return button("删除查询集合：查询甲");
}
function expectNoWrites(value: NalController) {
  expect(value.mutate).not.toHaveBeenCalled(); expect(value.removeCollection).not.toHaveBeenCalled();
  expect(value.editCollections).not.toHaveBeenCalled(); expect(value.refresh).not.toHaveBeenCalled();
}
function expectOriginalWrite(kind: Deletion, value: NalController) {
  if (kind === "collection") {
    expect(value.removeCollection).toHaveBeenCalledExactlyOnceWith("query-a");
    expect(value.mutate).not.toHaveBeenCalled();
  } else {
    expect(value.mutate).toHaveBeenCalledExactlyOnceWith({kind: kind === "concept" ? "delete_concept" : "delete_statement",
      id: kind === "concept" ? "concept-a" : "statement-a"}, 7);
    expect(value.removeCollection).not.toHaveBeenCalled();
  }
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear();
  selectAppRuntime({mode: "browser", storage: window.localStorage, gateways: {}, dispose() {}});
  vi.spyOn(notify, "error").mockReturnValue("error");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  if (originalConfirm) Object.defineProperty(window, "confirm", originalConfirm);
  else Reflect.deleteProperty(window, "confirm");
});

it.each(deletions)("awaits %s confirmation, blocks duplicate clicks, and releases cancellation without writing", async kind => {
  const value = controller(), pending = deferred<boolean>(), confirmation = vi.fn(() => pending.promise);
  Object.defineProperty(window, "confirm", {configurable: true, value: confirmation});
  await render(kind, value); const remove = await removeButton(kind);
  await act(async () => {remove.click(); remove.click();});
  expect(confirmation).toHaveBeenCalledExactlyOnceWith(prompts[kind]); expectNoWrites(value);
  await act(async () => pending.resolve(false)); expectNoWrites(value);
  expect(notify.error).not.toHaveBeenCalled();
  confirmation.mockResolvedValueOnce(true);
  await act(async () => (await removeButton(kind)).click());
  expectOriginalWrite(kind, value); expect(confirmation).toHaveBeenCalledTimes(2);
});

it.each(deletions)("retains Core confirmation diagnostics for %s and permits a new explicit attempt after failure", async kind => {
  const value = controller(), pending = deferred<boolean>(), confirmation = vi.fn(() => pending.promise);
  Object.defineProperty(window, "confirm", {configurable: true, value: confirmation});
  await render(kind, value); const remove = await removeButton(kind);
  await act(async () => remove.click()); expectNoWrites(value);
  await act(async () => pending.reject({code: "permission_denied", operationId: "confirmation-test", status: 403}));
  expectNoWrites(value);
  expect(notify.error).toHaveBeenCalledExactlyOnceWith(expect.stringContaining("PLATFORM_CONFIRMATION_FAILED"));
  for (const detail of ["code=permission_denied", "operationId=confirmation-test", "httpStatus=403"])
    expect(vi.mocked(notify.error).mock.calls[0]?.[0]).toContain(detail);
  confirmation.mockResolvedValueOnce(true);
  await act(async () => (await removeButton(kind)).click()); expectOriginalWrite(kind, value);
});

it.each(deletions)("keeps the original %s target and revision when the production selection refreshes during confirmation", async kind => {
  const value = controller(), pending = deferred<boolean>(), confirmation = vi.fn(() => pending.promise);
  Object.defineProperty(window, "confirm", {configurable: true, value: confirmation});
  await render(kind, value); const remove = await removeButton(kind);
  await act(async () => remove.click()); expectNoWrites(value);
  const next = {...controller(), state: {...value.state, revision: 19, statements: [{...statement, id: "statement-b"}]}, selectedCollectionId: "query-b"};
  await render(kind, next);
  if (kind === "concept") await act(async () => button("概念乙 · concept-b").click());
  if (kind === "collection") await act(async () => button("查询乙").click());
  await act(async () => pending.resolve(true));
  expectOriginalWrite(kind, value); expectNoWrites(next); expect(confirmation).toHaveBeenCalledTimes(1);
});

it.each(deletions)("keeps %s deletion locked until the accepted original submission settles", async kind => {
  const value = controller(), pending = deferred<boolean>(), confirmation = vi.fn(() => true);
  const write = kind === "collection" ? value.removeCollection : value.mutate;
  vi.mocked(write).mockReturnValueOnce(pending.promise);
  Object.defineProperty(window, "confirm", {configurable: true, value: confirmation});
  await render(kind, value); const remove = await removeButton(kind);
  await act(async () => remove.click()); expectOriginalWrite(kind, value);
  await act(async () => remove.click()); expect(confirmation).toHaveBeenCalledTimes(1);
  if (kind === "collection") expect(button("查询操作：查询甲").disabled).toBe(true);
  else expect(remove.disabled).toBe(true);
  await act(async () => pending.resolve(false));
  await act(async () => (await removeButton(kind)).click());
  expect(confirmation).toHaveBeenCalledTimes(2); expect(write).toHaveBeenCalledTimes(2);
});
