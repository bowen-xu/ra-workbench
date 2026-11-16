// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import type {Concept} from "@research-assistant/core/knowledge";
import {emptyNalState, type NalState} from "@research-assistant/core/nal";
import {notify} from "@research-assistant/workbench-sdk/feedback";
import {NalConceptEditor} from "../../../../../../../plugins/ra-literature-library/src/features/nal/NalConceptEditor";
import {NalExistingConceptEditor} from "../../../../../../../plugins/ra-literature-library/src/features/nal/NalExistingConceptEditor";
import {useNalAnnotation} from "../../../../../../../plugins/ra-literature-library/src/features/nal/useNalAnnotation";
import {useNal, type NalController} from "../../../../../../../plugins/ra-literature-library/src/features/nal/useNal";
import {configureLiteratureServices} from "../../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {createRuntimeLiteraturePort, selectAppRuntime, NotesProvider, useNotesClient, getNotesGateway}
  from "../../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";

const capabilities = ["literature.catalog-read", "literature.nal-bootstrap", "literature.nal-mutation", "literature.nal-annotation",
  "literature.nal-query-collections", "literature.concepts"].map(id => ({id, version: 1}));
const term = {id: "term", text: "Original", normalizedText: "original", language: "und", provenanceKind: "human_original" as const,
  revision: 1, createdAt: "created", updatedAt: "updated"};
const original: Concept = {id: "concept", preferredTerm: term, terms: [term], label: "Original", explanation: "", disambiguation: "",
  domain: "", tradition: "", period: "", provenanceKind: "human_original", revision: 1, definitionCount: 0, sourceCount: 0,
  createdAt: "created", updatedAt: "updated"};
let host: HTMLDivElement, root: Root, releases: (() => void)[];
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear();
  selectAppRuntime({mode: "browser", storage: window.localStorage, gateways: {}, dispose() {}});
  vi.spyOn(notify, "success").mockReturnValue("success"); vi.spyOn(notify, "error").mockReturnValue("error");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host); releases = [];
});
afterEach(async () => {await act(async () => root.unmount()); host.remove(); for (const release of releases.reverse()) release(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
function bind(invoke: (command: string, args: Record<string, unknown>) => Promise<unknown>) {
  releases.push(configureLiteratureServices(createRuntimeLiteraturePort("d".repeat(64), capabilities, () => undefined, invoke)));
}
async function input(selector: string, value: string) {
  const element = host.querySelector<HTMLInputElement>(selector)!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event("input", {bubbles: true}));
  });
}
async function submit() {await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));}

it.each(["read", "classify", "refresh"] as const)("retains the saved concept identity when %s fails and retries only the remaining original prefix", async failure => {
  let state: NalState = {...emptyNalState(), revision: 7}, failed = false;
  const invoke = vi.fn(async (command: string, args: Record<string, unknown>): Promise<unknown> => {
    if (command === "save_runtime_research_concept") return original;
    if (command === "read_runtime_nal_network") {
      if (failure === "read" && !failed) {failed = true; throw {code: "storage_error", message: "Synthetic read failure"};}
      return state;
    }
    if (command === "mutate_runtime_nal_network") {
      if (failure === "classify" && !failed) {failed = true; throw {code: "revision_conflict", message: "Synthetic classification failure"};}
      expect(args.mutation).toEqual({kind: "set_concept_kind", conceptId: original.id, conceptKind: "ordinary"});
      expect(args.expectedRevision).toBe(7); state = {...state, revision: 8}; return state;
    }
    throw new Error(`Unexpected command ${command}`);
  });
  bind(invoke);
  const refresh = vi.fn(async () => true).mockResolvedValueOnce(false), created = vi.fn();
  await act(async () => root.render(<NalConceptEditor embedded controller={{state, refresh}} onCreated={created} onClose={() => undefined}/>));
  await input('[aria-label="名称"]', "New concept"); await submit();
  expect(host.querySelector<HTMLInputElement>('[aria-label="名称"]')!.disabled).toBe(true);
  expect(host.textContent).toContain("重试会使用同一概念"); expect(created).not.toHaveBeenCalled();
  await submit();
  expect(created).toHaveBeenCalledExactlyOnceWith(original.id);
  expect(invoke.mock.calls.filter(([command]) => command === "save_runtime_research_concept")).toHaveLength(1);
  expect(invoke.mock.calls.filter(([command]) => command === "read_runtime_nal_network")).toHaveLength(failure === "refresh" ? 1 : 2);
  expect(invoke.mock.calls.filter(([command]) => command === "mutate_runtime_nal_network")).toHaveLength(failure === "classify" ? 2 : 1);
  expect(refresh).toHaveBeenCalledTimes(2);
});

it("keeps the saved revision in the original existing-concept editor when refresh fails", async () => {
  let saved = original;
  const invoke = vi.fn(async (command: string, args: Record<string, unknown>): Promise<unknown> => {
    if (command === "search_runtime_research_concepts") {expect(args.query).toBe(""); return [original];}
    if (command === "save_runtime_research_concept") {
      const draft = args.draft as {id: string; expectedRevision: number; label: string};
      expect(draft.id).toBe(original.id); expect(draft.expectedRevision).toBe(saved.revision);
      saved = {...saved, label: draft.label, revision: saved.revision + 1}; return saved;
    }
    throw new Error(`Unexpected command ${command}`);
  });
  bind(invoke); const close = vi.fn(), refresh = vi.fn(async () => true).mockResolvedValueOnce(false);
  await act(async () => root.render(<NalExistingConceptEditor id={original.id} controller={{state: emptyNalState(), refresh}} onClose={close}/>));
  await input('[aria-label="已有概念名称"]', "Updated"); await submit();
  expect(close).not.toHaveBeenCalled(); expect(saved.revision).toBe(2);
  await input('[aria-label="已有概念名称"]', "Updated again"); await submit();
  expect(saved.revision).toBe(3); expect(close).toHaveBeenCalledOnce();
  expect(invoke.mock.calls.filter(([command]) => command === "save_runtime_research_concept").map(([, args]) =>
    (args.draft as {expectedRevision: number}).expectedRevision)).toEqual([1, 2]);
});

it("opens the original existing annotation's same trashed Core Note without creating or restoring a document", async () => {
  const notes = getNotesGateway(), page = await notes.createPage({title: "Existing annotation"});
  await notes.setPageStatus({id: page.page.id, expectedRevision: page.page.revision, status: "trashed"});
  const before = await notes.loadDocument(page.page.id);
  const annotation = {id: "existing", targetKind: "concept", targetId: original.id, notePageId: page.page.id};
  const invoke = vi.fn(async (command: string): Promise<unknown> => {
    if (command !== "ensure_runtime_nal_annotation") throw new Error(`Unexpected command ${command}`);
    return annotation;
  });
  bind(invoke); let annotate!: ReturnType<typeof useNalAnnotation>, version!: () => number;
  const refresh = vi.fn(async () => false);
  function Consumer() {annotate = useNalAnnotation({refresh}); version = useNotesClient().getVersion; return null;}
  await act(async () => root.render(<NotesProvider gateway={notes}><Consumer/></NotesProvider>));
  const open = vi.fn(); window.addEventListener("research-assistant:open-annotation-note", open);
  try {
    const initialVersion = version();
    for (let i = 0; i < 2; i++) await act(async () => annotate({kind: "concept", id: original.id, label: "Existing annotation"}));
    expect(invoke).toHaveBeenCalledTimes(2); expect(refresh).toHaveBeenCalledTimes(2); expect(version()).toBeGreaterThan(initialVersion);
    expect(open).toHaveBeenCalledTimes(2);
    expect(open.mock.calls.map(([event]) => (event as CustomEvent).detail)).toEqual([
      {pageId: page.page.id, kind: "concept"}, {pageId: page.page.id, kind: "concept"},
    ]);
    expect(await notes.loadDocument(page.page.id)).toEqual(before);
    expect((await notes.listPages({status: "trashed"})).map(row => row.id)).toEqual([page.page.id]);
  } finally {window.removeEventListener("research-assistant:open-annotation-note", open);}
});

it("keeps a successful relation mutation committed when its original refresh fails", async () => {
  let failRead = false;
  const invoke = vi.fn(async (command: string): Promise<unknown> => {
    if (command === "mutate_runtime_nal_network") {failRead = true; return {...emptyNalState(), revision: 1};}
    if (command === "read_runtime_nal_network") {if (failRead) throw {code: "storage_error", message: "Synthetic refresh failure"}; return emptyNalState();}
    if (command === "list_runtime_literature") return [];
    if (command === "read_runtime_nal_query_collections") return {revision: 1, definitions: [], cache: null, legacyImported: true};
    if (command === "save_runtime_nal_query_collections") return {revision: 2, definitions: [], cache: null, legacyImported: true};
    throw new Error(`Unexpected command ${command}`);
  });
  bind(invoke); let controller!: NalController;
  const items: readonly never[] = [];
  function Consumer() {controller = useNal(items); return null;}
  await act(async () => root.render(<Consumer/>));
  let result: boolean | undefined;
  await act(async () => {result = await controller.mutate({kind: "delete_statement", id: "statement"}, 0);});
  expect(result).toBe(true); expect(controller.ready).toBe(false);
  expect(notify.success).toHaveBeenCalledWith("关系已保存");
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("Synthetic refresh failure"));
  expect(invoke.mock.calls.filter(([command]) => command === "mutate_runtime_nal_network")).toHaveLength(1);
});

it("settles an accepted annotation after unmount without reopening a document or replaying the request", async () => {
  const notes = getNotesGateway(), page = await notes.createPage({title: "Late existing annotation"});
  let finish!: (value: unknown) => void;
  const invoke = vi.fn(async () => new Promise(accept => {finish = accept;}));
  bind(invoke); let annotate!: ReturnType<typeof useNalAnnotation>, version!: () => number;
  const refresh = vi.fn(async () => true);
  function Consumer() {annotate = useNalAnnotation({refresh}); version = useNotesClient().getVersion; return null;}
  await act(async () => root.render(<NotesProvider gateway={notes}><Consumer/></NotesProvider>));
  const initialVersion = version(), open = vi.fn(); window.addEventListener("research-assistant:open-annotation-note", open);
  try {
    let pending!: Promise<void>;
    await act(async () => {pending = annotate({kind: "concept", id: original.id, label: "Late existing annotation"});});
    await act(async () => root.render(null));
    await act(async () => {finish({id: "existing", targetKind: "concept", targetId: original.id, notePageId: page.page.id}); await pending;});
    expect(version()).toBeGreaterThan(initialVersion); expect(refresh).not.toHaveBeenCalled(); expect(open).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenCalledOnce(); expect((await notes.listPages()).map(row => row.id)).toEqual([page.page.id]);
  } finally {window.removeEventListener("research-assistant:open-annotation-note", open);}
});
