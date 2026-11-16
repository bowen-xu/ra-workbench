// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createStructuredDocumentAdapter, HbsSession } from "@research-assistant/core/hbs";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import { HbsStructuredEditor } from "../../../../../../../apps/web/src/core/components/hbs/HbsStructuredEditor";
import { directTextDiff } from "../../../../../../../apps/web/src/core/components/hbs/codeMirrorHbsPolicy";
import { createMockStructuredResources } from "../../../../../../../../integration/web/src/app/runtime/mock/mockStructuredResources";
import type { AppHbsDocumentGateway } from "../../../../../../../apps/web/src/core/services/hbsDocumentGateway";
import { PortraitHistoryProvider, portraitHistoryKeyDown, usePortraitEditHistory } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/PortraitEditHistory";
import { StructuredResourceReview } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/StructuredResourceReview";

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
vi.mock("../../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
const clean = "视觉识别能否被构造成逻辑推理？1213\n\n方法原文";
const candidate = "视觉识别能否被构造成逻辑推理？\n\n方法原文";
const typed = "11111视觉识别能否被构造成逻辑推理？\n\n方法人工输入";
const expectedOld = "视觉识别能否被构造成逻辑推理？1213\n\n方法人工输入";
const body = (text: string) => ({ kind: "paper_overview_unit_v1", schemaVersion: 1,
  fields: [{ field: "question", statements: [{ id: "q", text, evidenceItemIds: [] }] }] });
const disposals: (() => Promise<void>)[] = [];
afterEach(async () => { for (const dispose of disposals.splice(0)) await dispose(); });

async function setup(mode: "app" | "adapter", { pending = true, panel = false }: { pending?: boolean; panel?: boolean } = {}) {
  const values = new Map<string, string>();
  const storage: Storage = { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); }, clear: () => values.clear() };
  const gateway = { ...createMockStructuredResources(storage), getSources: async () => [] };
  const raw: AppHbsDocumentGateway = { read: (_kind, id) => gateway.hbs.read(id),
    commit: (_kind, id, request) => gateway.hbs.commit(id, request),
    readPreservation: (_kind, id) => gateway.hbs.readPreservation(id),
    savePreservation: (_kind, id, value) => gateway.hbs.savePreservation(id, value) };
  const target = (await gateway.list("paper")).find(value => value.kind === "structuredReading")!;
  const initial = (await gateway.write({ resourceId: target.id, expectedRevision: 0, requestId: "initial",
    editorKind: "human", content: JSON.stringify(body(clean)) })).resource;
  const changed = pending ? (await gateway.write({ resourceId: target.id, expectedRevision: initial.revision,
    requestId: "external", editorKind: "agent", content: JSON.stringify(body(candidate)) })).resource : initial;
  // The adapter route injects a session into the same production card editor.
  const adapter = createStructuredDocumentAdapter(directTextDiff);
  let authority = { version: changed.revision, saved: changed.content, baseline: initial.content };
  const session = await HbsSession.open({ adapter, requestId: n => `adapter-${n}`,
    preservation: { load: () => undefined, save: () => {} }, port: {
      read: async () => authority,
      commit: async request => { authority = { version: request.expectedVersion + 1, saved: request.saved, baseline: request.baseline }; return authority; } } });
  let setVisible: (value: boolean) => void = () => {};
  function Host() {
    const [resource, setResource] = useState<StructuredResource>(changed), [visible, show] = useState(true);
    setVisible = show;
    const history = usePortraitEditHistory();
    return <main onKeyDown={event => { if (panel) portraitHistoryKeyDown(event, history, false); }}>
      <button type="button">切到其他内容</button>
      {visible && (mode === "app" ? <StructuredResourceReview resource={resource} gateway={gateway} hbsGateway={raw} onChanged={setResource}/>
        : <HbsStructuredEditor session={session}/>)}</main>;
  }
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  disposals.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); });
  await act(async () => root.render(panel ? <PortraitHistoryProvider><Host/></PortraitHistoryProvider> : <Host/>));
  const key = async (redo = false) => { await act(async () => host.querySelector(".structured-resource-review")!
    .dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, shiftKey: redo, bubbles: true, cancelable: true }))); };
  const edit = async (text: string) => {
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="编辑question"]')!.click());
    const input = host.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, text);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    return input;
  };
  const leave = async () => {
    await act(async () => host.querySelector<HTMLButtonElement>("main > button")!.focus());
    await vi.waitFor(() => expect(host.querySelector("textarea")).toBeNull());
  };
  const switchBack = async () => { await act(async () => setVisible(false)); await act(async () => setVisible(true)); };
  const external = async (saved: unknown) => {
    authority = { ...authority, version: authority.version + 1, saved };
    await act(async () => session.syncLatest());
  };
  return { host, edit, leave, key, switchBack, external };
}

it.each(["app", "adapter"] as const)("%s uses the real card and keeps old stable for input in the new line", async mode => {
  const f = await setup(mode);
  expect(f.host.querySelector('[data-field-id="question"]')?.classList.contains("reading-category-card")).toBe(true);
  await f.edit(typed);
  expect(f.host.querySelector('[aria-label="修改前"]')?.textContent).toBe(expectedOld);
  await f.leave();
  expect(f.host.querySelector('[aria-label="修改前"]')?.textContent).toBe(expectedOld);
  expect(f.host.querySelector('[aria-label="修改后"]')?.textContent).toBe(typed);
});
it.each(["app", "adapter"] as const)("%s retains typed new and its original baseline through blur, switching and undo/redo", async mode => {
  const f = await setup(mode);
  await f.edit(typed); await f.leave(); await f.switchBack();
  await f.key();
  expect(f.host.querySelector('[aria-label="修改前"]')?.textContent).toBe(clean);
  expect(f.host.querySelector('[aria-label="修改后"]')?.textContent).toBe(candidate);
  await f.key(true);
  expect(f.host.querySelector('[aria-label="修改前"]')?.textContent).toBe(expectedOld);
  expect(f.host.querySelector('[aria-label="修改后"]')?.textContent).toBe(typed);
});
it("keeps card input in the enclosing panel history after switching away and back", async () => {
  const f = await setup("app", { pending: false, panel: true });
  await f.edit("普通人工修改"); await f.leave(); await f.switchBack();
  await f.key(); expect(f.host.textContent).toContain(clean);
  expect(f.host.querySelector(".structured-card-diff")).toBeNull();
  await f.key(true); expect(f.host.textContent).toContain("普通人工修改");
  expect(f.host.querySelector(".structured-card-diff")).toBeNull();
});
it.each(["app", "adapter"] as const)("%s saves candidate edits on Escape while keeping them reviewable", async mode => {
  const f = await setup(mode);
  const input = await f.edit(typed);
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(f.host.querySelector("textarea")).toBeNull();
  expect(f.host.querySelector('[aria-label="修改前"]')?.textContent).toBe(expectedOld);
  expect(f.host.querySelector('[aria-label="修改后"]')?.textContent).toBe(typed);
});
it.each(["app", "adapter"] as const)("%s keeps ordinary edits ordinary after blur and repeated undo/redo", async mode => {
  const f = await setup(mode, { pending: false });
  await f.edit("普通人工输入"); await f.leave(); await f.switchBack();
  for (let index = 0; index < 2; index++) {
    await f.key(); expect(f.host.textContent).toContain(clean);
    expect(f.host.querySelector(".structured-card-diff")).toBeNull();
    await f.key(true); expect(f.host.textContent).toContain("普通人工输入");
    expect(f.host.querySelector(".structured-card-diff")).toBeNull();
  }
});
it.each([{ mode: "app", panel: false }, { mode: "adapter", panel: false }, { mode: "app", panel: true }] as const)("$mode uses one history while editing and after leaving the textarea (panel: $panel)", async ({ mode, panel }) => {
  const f = await setup(mode, { panel });
  const input = await f.edit(typed);
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true, cancelable: true })));
  expect(input.value).toBe(candidate);
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true })));
  expect(input.value).toBe(typed);
  await f.leave(); await f.key();
  expect(f.host.querySelector('[aria-label="修改后"]')?.textContent).toBe(candidate);
  await f.key(true);
  expect(f.host.querySelector('[aria-label="修改后"]')?.textContent).toBe(typed);
  expect(f.host.querySelector('[aria-label="修改前"]')?.textContent).toBe(expectedOld);
});
it("saves on Escape without losing a compatible external field after undo/redo inside the textarea", async () => {
  const f = await setup("adapter");
  const input = await f.edit(typed);
  const incoming = body(candidate);
  await f.external({ ...incoming, fields: [...incoming.fields,
    { field: "remote", statements: [{ id: "remote", text: "外部新增字段", evidenceItemIds: [] }] }] });
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true, cancelable: true })));
  expect(input.value).toBe(candidate);
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true })));
  expect(input.value).toBe(typed);
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(f.host.querySelector('[data-field-id="remote"]')?.textContent).toContain("外部新增字段");
  const question = f.host.querySelector('[data-field-id="question"]')!;
  expect(question.querySelector('[aria-label="修改前"]')?.textContent).toBe(expectedOld);
  expect(question.querySelector('[aria-label="修改后"]')?.textContent).toBe(typed);
});
