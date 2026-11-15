// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { undo, redo } from "@codemirror/commands";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { HbsClient, MemoryHbsPreservationStore, type HbsPreservationStore, type HbsSnapshot } from "@research-assistant/core/hbs";
import { HbsWireAuthority, HbsWireClient, MemoryHbsRepository, type HbsWireEncoding } from "@research-assistant/data/hbs";
import { HbsTextEditor } from "./HbsTextEditor";
import { createCodeMirrorHbsPolicy, directTextDiff } from "./codeMirrorHbsPolicy";
import { textChanges } from "../editor-diff/textDiffCommands";
import type { EditorReviewConfiguration } from "../editor-diff/EditorReviewActions";
import { BrowserHbsPreservationStore } from "./browserHbsPreservation";
import { openHbsTextClient } from "./openHbsTextClient";

const initial: HbsSnapshot = { version: 1, saved: [{ id: "doc", label: "Document", text: "heading\n\nvalue 80%\n\nmethod\nlast" }], baseline: [{ id: "doc", label: "Document", text: "heading\n\nvalue 80%\n\nmethod\nlast" }] };
let container: HTMLDivElement, root: ReturnType<typeof createRoot>, repository: MemoryHbsRepository, port: HbsWireClient, client: HbsClient;
let store: MemoryHbsPreservationStore, wires: string[], incarnation = 0;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); store = new MemoryHbsPreservationStore(); wires = [];
  repository = new MemoryHbsRepository(initial); container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); client?.dispose(); port?.dispose(); container.remove(); vi.unstubAllGlobals(); });
async function open(mode: HbsWireEncoding = { type: "patch", calculate: directTextDiff }, preservation: HbsPreservationStore = store, configuration?: EditorReviewConfiguration) {
  await act(async () => {
    client?.dispose(); port?.dispose(); const authority = new HbsWireAuthority(repository);
    port = await HbsWireClient.connect({ request: async wire => { wires.push(wire); return authority.handle(wire); } }, mode);
    const id = ++incarnation;
    client = await openHbsTextClient({ port, documentId: "doc", policy: createCodeMirrorHbsPolicy(), preservation, requestId: sequence => `${id}:${sequence}` });
    root.render(<HbsTextEditor key={id} client={client} path="example.txt" {...(configuration ? { reviewConfiguration: configuration } : {})}
      fileActions={{ reveal: async () => {}, saveAs: async () => {} }} />);
  });
}
function editor(): EditorView { return EditorView.findFromDOM(container.querySelector<HTMLElement>(".cm-editor")!)!; }
function h(): string { return editor().state.doc.toString(); }
async function edit(value: string) { await act(async () => editor().dispatch({ changes: textChanges(h(), value), userEvent: "input.type" })); }
async function external(value: string) { const current = repository.read(); await repository.commit(current.version, [{ id: "doc", saved: value, baseline: current.baseline[0]!.text }]); }
async function candidate() { await external(initial.saved[0]!.text.replace("80%", "85%")); await act(async () => client.syncLatest()); }
async function conflict() { await candidate(); await edit(h().replace("85%", "91%")); await external(initial.saved[0]!.text.replace("80%", "90%")); await act(async () => client.save()); }
async function click(label: string) {
  const button = [...container.querySelectorAll("button")].find(button => button.textContent === label);
  if (!button) throw new Error(`Missing ${label}`); await act(async () => button.click());
}

it.each(["patch", "snapshot"] as const)("shares review, conflict and native history through %s transport", async encoding => {
  await open(encoding === "patch" ? { type: encoding, calculate: directTextDiff } : { type: encoding }); await conflict();
  expect(container.querySelector('[aria-label="incoming"]')?.textContent).toContain("90%");
  expect(client.getSnapshot().preserved).toContain("91%"); expect(repository.read().baseline[0]!.text).toContain("80%");
  await click("Both"); expect(h()).toContain("value 90%\nvalue 91%");
  await act(async () => { expect(undo(editor())).toBe(true); }); expect(h()).toContain("91%");
  expect(container.querySelector('[aria-label="incoming"]')).not.toBeNull();
  await act(async () => { expect(redo(editor())).toBe(true); }); expect(h()).toContain("90%\nvalue 91%");
  await click("Keep"); expect(repository.read().saved[0]!.text).toBe(h()); expect(repository.read().baseline[0]!.text).toBe(h());
  expect(wires.some(wire => wire.includes(`"encoding":"${encoding}"`))).toBe(true);
});
it("saves ordinary input into S/B while edited new stays pending", async () => {
  await open(); await candidate(); await edit(h().replace("method", "my method").replace("85%", "91%")); await act(async () => client.save());
  expect(repository.read().baseline[0]!.text).toContain("my method"); expect(repository.read().baseline[0]!.text).toContain("80%");
  expect(repository.read().saved[0]!.text).toContain("91%"); expect(getChunks(editor().state)?.chunks).toHaveLength(1);
});
it.each(["patch", "snapshot"] as const)("undoes and redoes committed Keep through new %s commits", async encoding => {
  await open(encoding === "patch" ? { type: encoding, calculate: directTextDiff } : { type: encoding }); await candidate();
  await click("Keep"); const keptVersion = repository.read().version; expect(repository.read().baseline[0]!.text).toContain("85%");
  await act(async () => { expect(undo(editor())).toBe(true); });
  expect(repository.read().baseline[0]!.text).toContain("80%"); expect(h()).toContain("85%");
  expect(getChunks(editor().state)?.chunks).toHaveLength(1); expect(repository.read().version).toBeGreaterThan(keptVersion);
  await act(async () => { expect(redo(editor())).toBe(true); }); expect(repository.read().baseline[0]!.text).toContain("85%");
  expect(repository.read().version).toBeGreaterThan(keptVersion + 1);
});
it("restores S′ through an asynchronous store without changing the authority", async () => {
  await open(); await conflict(); const server = repository.read();
  const asyncStore: HbsPreservationStore = { durable: true, load: async () => store.load(), save: async value => store.save(value) };
  await open({ type: "snapshot" }, asyncStore);
  expect(h()).toContain("91%"); expect(getOriginalDoc(editor().state).toString()).toContain("80%");
  expect(container.querySelector('[aria-label="incoming"]')?.textContent).toContain("90%"); expect(repository.read()).toEqual(server);
});
it("gates submission on an asynchronous checkpoint and retains later typing", async () => {
  let release: (() => void) | undefined, blocked = false;
  const asyncStore: HbsPreservationStore = { durable: true, load: () => store.load(), save: value => {
    if (!blocked) { store.save(value); return; }
    return new Promise<void>(resolve => { release = () => { store.save(value); blocked = false; resolve(); }; });
  } };
  await open({ type: "snapshot" }, asyncStore); await candidate(); await edit(h().replace("85%", "91%")); blocked = true;
  await act(async () => client.save()); expect(wires.filter(wire => wire.includes('"commit"'))).toHaveLength(0);
  await edit(h().replace("91%", "92%")); await act(async () => release?.());
  expect(store.load()?.content).toContain("92%"); expect(repository.read().saved[0]!.text).toContain("92%");
  expect(repository.read().baseline[0]!.text).toContain("80%");
});
it("checkpoints review before an asynchronous store permits the Keep commit", async () => {
  let blocked = false, release: (() => void) | undefined;
  const asyncStore: HbsPreservationStore = { durable: true, load: () => store.load(), save: value => {
    if (!blocked) { store.save(value); return; }
    return new Promise<void>(resolve => { release = () => { store.save(value); blocked = false; resolve(); }; });
  } };
  await open({ type: "snapshot" }, asyncStore); await candidate(); blocked = true; await click("Keep");
  expect(wires.filter(wire => wire.includes('"commit"'))).toHaveLength(0); expect(h()).toContain("85%");
  await act(async () => release?.()); expect(repository.read().baseline[0]!.text).toContain("85%"); expect(store.load()?.content).toBe(h());
});
it("configures conflict choices, labels, bulk controls and read-only incoming without replacing the editor", async () => {
  await open({ type: "snapshot" }, store, { conflictActions: ["incoming"], labels: { incoming: "采用传入", keep: "确认" }, showBulk: false,
    renderIncoming: text => <strong>{text}</strong> }); await conflict();
  const instance = editor(); expect(container.textContent).not.toContain("Both"); expect(container.textContent).not.toContain("Keep all");
  expect(container.querySelector('[aria-label="incoming"] strong')?.textContent).toContain("90%");
  expect(container.querySelector('[aria-label="incoming"]')?.closest('[contenteditable="false"]')).not.toBeNull();
  await click("采用传入"); expect(editor()).toBe(instance); expect(h()).toContain("90%");
  expect(repository.read().baseline[0]!.text).toContain("80%"); await click("确认"); expect(repository.read().baseline[0]!.text).toContain("90%");
});
it("uses the reusable browser preservation adapter and default client factory independently of the reference", async () => {
  const browser = new BrowserHbsPreservationStore({ storage: window.localStorage, unitId: "doc", identity: { scope: "hbs-test", resourceId: "example", slot: "draft" } });
  browser.clear();
  try {
    await open({ type: "snapshot" }, browser); await conflict(); const expected = h();
    await open({ type: "snapshot" }, browser); expect(h()).toBe(expected);
    expect(container.querySelector('[aria-label="incoming"]')?.textContent).toContain("90%");
    expect(client.preservationDurable()).toBe(true);
  } finally { browser.clear(); }
});
