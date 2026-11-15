import { afterEach, expect, it } from "vitest";
import { HbsSession, DocumentVersionConflict, createTextDocumentAdapter, createNoteDocumentAdapter, createStructuredDocumentAdapter,
  type HbsDocumentAdapter, type DocumentCheckpoint, type DocumentCommit, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { notePageId, noteBlockId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { directTextDiff } from "./codeMirrorHbsPolicy";

const settle = async () => { for (let n = 0; n < 160; n++) await Promise.resolve(); };
const sessions: { dispose(): void }[] = [];
afterEach(() => sessions.splice(0).forEach(session => session.dispose()));
const source = "重复实验 3 次。\n\n末尾";
const codecs: { name: string; adapter: HbsDocumentAdapter<unknown>; encode(text: string): unknown; text(value: unknown): string }[] = [
  { name: "text", adapter: createTextDocumentAdapter(directTextDiff) as HbsDocumentAdapter<unknown>, encode: text => text, text: value => String(value) },
  { name: "note", adapter: createNoteDocumentAdapter(directTextDiff) as HbsDocumentAdapter<unknown>,
    encode: text => ({ schemaVersion: 1, pageId: notePageId("baseline-test"), blocks: [{ id: noteBlockId("nested"), type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [{ type: "text", text, styles: [] }] } }] }),
    text: value => { const content = (value as NoteDocumentV1).blocks[0]!.content; return content.kind === "inline" ? content.items.map(item => item.type === "text" ? item.text : "").join("") : ""; } },
  { name: "structured", adapter: createStructuredDocumentAdapter(directTextDiff), encode: text => ({ fields: [{ id: "method", statements: [{ id: "s1", text }] }] }),
    text: value => (value as { fields: { statements: { text: string }[] }[] }).fields[0]!.statements[0]!.text },
];
async function fixture(codec: typeof codecs[number], initial = source) {
  const { adapter, encode, text } = codec;
  let server: DocumentSnapshot<unknown> = { version: 1, saved: encode(initial), baseline: encode(initial) };
  let checkpoint: DocumentCheckpoint<unknown> | undefined, hold = false, holdReceipt = false, fail = false;
  const requests: DocumentCommit<unknown>[] = [], waiting: (() => void)[] = [];
  const options = { adapter, requestId: (n: number) => `align-${n}`, preservation: { load: () => checkpoint, save: (value: DocumentCheckpoint<unknown>) => { checkpoint = structuredClone(value); } },
    port: { read: async () => server, commit: async (request: DocumentCommit<unknown>) => {
      requests.push(request);
      if (hold) await new Promise<void>(resolve => waiting.push(resolve));
      if (fail) { fail = false; throw new Error("storage failed"); }
      if (request.expectedVersion !== server.version) throw new DocumentVersionConflict(server, request.id);
      server = { version: server.version + 1, saved: request.saved, baseline: request.baseline };
      const receipt = server;
      if (holdReceipt) await new Promise<void>(resolve => waiting.push(resolve));
      return receipt;
    } } };
  const session = await HbsSession.open(options); sessions.push(session);
  return { session, options, requests, state: session.getSnapshot,
    h: () => text(session.getSnapshot().content), bp: () => text(session.getSnapshot().baselineProjection),
    s: () => text(server.saved), b: () => text(server.baseline), get version() { return server.version; },
    edit: (value: string, ordinary = true) => session.edit(encode(value), ordinary ? encode(value) : session.getSnapshot().baselineProjection),
    external: (value: string, confirmed = false) => { server = { ...server, version: server.version + 1, saved: encode(value), ...(confirmed ? { baseline: encode(value) } : {}) }; },
    hold: () => { hold = true; }, holdAck: () => { holdReceipt = true; }, release: () => { hold = false; holdReceipt = false; waiting.splice(0).forEach(resolve => resolve()); }, fail: () => { fail = true; } };
}
for (const codec of codecs) {
  it(`${codec.name}: commits ordinary B′ before comparison and Undo restores that red baseline`, async () => {
    const f = await fixture(codec), mine = source.replace("3 次", "143 次"); f.edit(mine);
    f.external("外部修改 v150\n\n末尾"); await f.session.syncLatest(); await settle();
    expect(f.b()).toBe(mine); expect(f.bp()).toBe(mine); expect(f.h()).toBe(f.s()); expect(f.state().conflicts).toHaveLength(0);
    expect(f.requests).toHaveLength(1); expect(codec.text(f.requests[0]!.saved)).toBe(f.s());
    f.session.resolveMany(f.state().changes.map(change => change.id), "undo"); await settle(); expect(f.h()).toBe(mine); expect(f.s()).toBe(mine);
  });
  it(`${codec.name}: identical ordinary and S results naturally leave no Diff`, async () => {
    const f = await fixture(codec), mine = source.replace("3 次", "143 次"); f.edit(mine); f.external(mine);
    await f.session.syncLatest(); await settle(); expect(f.b()).toBe(mine); expect(f.state().changes).toHaveLength(0); expect(f.state().conflicts).toHaveLength(0);
  });
  it(`${codec.name}: edited new retains its complete human branch and uses B′ for Undo`, async () => {
    const f = await fixture(codec); f.external(source.replace("3 次", "4 次")); await f.session.syncLatest();
    const mine = source.replace("3 次", "143 次"); f.edit(mine, false); await f.session.save();
    f.external("外部修改 v150\n\n末尾"); await f.session.syncLatest(); await settle();
    expect(f.b()).toBe(source); expect(f.h()).toBe(mine); expect(f.state().conflicts.length).toBeGreaterThan(0);
    f.session.resolve(f.state().conflicts[0]!.id, "undo"); await settle(); expect(f.h()).toBe(source); expect(f.s()).toBe(source);
  });
  it(`${codec.name}: keeps actual concurrent B conflicts instead of overwriting them`, async () => {
    const f = await fixture(codec); f.edit(source.replace("3 次", "143 次")); f.external(source.replace("3 次", "5 次"), true);
    await f.session.syncLatest(); await settle(); expect(f.b()).toContain("5 次"); expect(f.bp()).toContain("143 次"); expect(f.state().conflicts.length).toBeGreaterThan(0); expect(f.requests).toHaveLength(0);
  });
  it(`${codec.name}: holds immutable baseline writes, retries a CAS race and preserves later typing`, async () => {
    const f = await fixture(codec); f.hold(); f.edit(source.replace("3 次", "143 次")); f.external("外部修改 v150\n\n末尾");
    await f.session.syncLatest(); await settle(); expect(f.b()).toBe(source); expect(f.h()).toContain("143 次");
    expect(f.requests).toHaveLength(1);
    f.edit(source.replace("3 次", "144 次")); f.external("外部修改 v151\n\n末尾"); f.release(); await settle();
    expect(codec.text(f.requests[0]!.baseline)).toContain("143 次");
    expect(f.b()).toContain("144 次"); expect(f.bp()).toContain("144 次"); expect(f.h()).toBe("外部修改 v151\n\n末尾"); expect(f.state().conflicts).toHaveLength(0);
  });
  it(`${codec.name}: waits for the actual ACK even when a sync reads the committed baseline`, async () => {
    const f = await fixture(codec); f.holdAck(); f.edit(source.replace("3 次", "143 次")); f.external("外部修改 v150\n\n末尾");
    await f.session.syncLatest(); await settle(); expect(f.b()).toContain("143 次"); expect(f.h()).toContain("143 次");
    await f.session.syncLatest(); expect(f.h()).toContain("143 次"); expect(f.state().pending).toBe(true);
    f.edit(source.replace("3 次", "144 次")); f.release(); await settle();
    expect(f.b()).toContain("144 次"); expect(f.h()).toBe(f.s()); expect(f.state().pending).toBe(false);
  });
  it(`${codec.name}: retries an unknown result with the same request and reopens without writing`, async () => {
    const f = await fixture(codec); f.fail(); f.edit(source.replace("3 次", "143 次")); f.external("外部修改 v150\n\n末尾");
    await f.session.syncLatest(); await settle(); expect(f.state().message).toContain("MODEL_RESULT_UNKNOWN"); expect(f.h()).toContain("143 次");
    const version = f.version, reopened = await HbsSession.open(f.options); sessions.push(reopened); await settle();
    expect(f.version).toBe(version); expect(codec.text(reopened.getSnapshot().content)).toContain("143 次"); reopened.dispose();
    await f.session.save(); await settle(); expect(f.requests[1]!.id).toBe(f.requests[0]!.id);
    expect(f.h()).toBe(f.s()); expect(f.b()).toContain("143 次"); expect(f.state().conflicts).toHaveLength(0);
  });
  it(`${codec.name}: aligns ordinary input without committing human new into B`, async () => {
    const initial = "普通标题\n\n分隔\n\n数值80%", f = await fixture(codec, initial);
    f.external(initial.replace("80%", "85%")); await f.session.syncLatest();
    const baseline = initial.replace("普通标题", "我的标题"), content = baseline.replace("80%", "91%");
    f.session.edit(codec.encode(content), codec.encode(baseline)); f.external(initial.replace("80%", "90%"));
    await f.session.syncLatest(); await settle(); expect(f.b()).toBe(baseline); expect(f.h()).toBe(content);
    expect(f.s()).toBe(baseline.replace("80%", "90%")); expect(f.state().conflicts.length).toBeGreaterThan(0);
  });
}
