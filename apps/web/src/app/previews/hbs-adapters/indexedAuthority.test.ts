// @vitest-environment happy-dom
import { beforeEach, expect, it } from "vitest";
import { createNoteDocumentAdapter, createStructuredDocumentAdapter, createTextDocumentAdapter, HbsSession,
  type DocumentCheckpoint, type DocumentSnapshot, type HbsDocumentAdapter } from "@research-assistant/core/hbs";
import { createDocumentPort, createDocumentPreservationPort, decodeDocumentSnapshot } from "@research-assistant/data/hbs";
import { configurePreviewDatabase, PreviewStorageError } from "../../../core/runtime/previewIndexedDatabase";
import { createMemoryPreviewDatabase } from "../../runtime/testing/memoryPreviewDatabase";
import { createIndexedHbsAuthority, HBS_ADAPTER_DATABASE_PREFIX } from "./indexedAuthority";
import { noteFixture, structuredFixture, textFixture } from "./fixtures";

const diff = (a: string, b: string) => a === b ? [] : [{fromA: 0, toA: a.length, fromB: 0, toB: b.length}];
const cases = [
  {adapter: createTextDocumentAdapter(diff) as HbsDocumentAdapter<unknown>, fixture: textFixture},
  {adapter: createNoteDocumentAdapter(diff) as HbsDocumentAdapter<unknown>, fixture: noteFixture},
  {adapter: createStructuredDocumentAdapter(diff), fixture: structuredFixture},
];
let database: ReturnType<typeof createMemoryPreviewDatabase>;
beforeEach(() => { database = createMemoryPreviewDatabase(); configurePreviewDatabase(database.open); });
function changed(adapter: HbsDocumentAdapter<unknown>, _fixture: unknown, text = "Changed") {
  if (adapter.kind === "text") return text;
  if (adapter.kind === "note") return {...noteFixture, blocks: [{...noteFixture.blocks[0]!, content: {kind: "inline", items: [{type: "text", text, styles: []}]}}]};
  return {...structuredFixture, profileId: text};
}
function checkpoint(adapter: HbsDocumentAdapter<unknown>, observed: DocumentSnapshot<unknown>, content: unknown): DocumentCheckpoint<unknown> {
  return {format: 1, kind: adapter.kind, observed, ancestor: observed.saved, baselineAncestor: observed.baseline, content, ordinary: []};
}

for (const {adapter, fixture} of cases) {
  it(`${adapter.kind}: seeds only an empty isolated database and reopens the committed S/B/version`, async () => {
    const first = await createIndexedHbsAuthority(adapter, fixture);
    const snapshot = decodeDocumentSnapshot(await first.call("read"), adapter);
    expect(snapshot).toEqual({version: 1, saved: fixture, baseline: fixture});
    const saved = changed(adapter, fixture);
    await first.call("commit", {id: "save", expectedVersion: 1, saved, baseline: fixture}); first.close();
    const reopened = await createIndexedHbsAuthority(adapter, changed(adapter, fixture, "Different bootstrap"));
    expect(await reopened.call("read")).toEqual({version: 2, saved, baseline: fixture});
    expect(database.metrics.initializations).toBe(1); reopened.close();
  });

  it(`${adapter.kind}: retains pending review and S′ across close/reopen without treating preservation as Keep`, async () => {
    const first = await createIndexedHbsAuthority(adapter, fixture);
    const saved = changed(adapter, fixture, "External candidate"), content = changed(adapter, fixture, "My candidate edit");
    await first.call("commit", {id: "external", expectedVersion: 1, saved, baseline: fixture});
    const observed = decodeDocumentSnapshot(await first.call("read"), adapter);
    const preserved = checkpoint(adapter, observed, content);
    expect(await first.call("savePreservation", preserved)).toEqual({revision: 1}); first.close();
    const reopened = await createIndexedHbsAuthority(adapter, fixture);
    expect(await reopened.call("readPreservation")).toEqual(preserved);
    expect(await reopened.call("read")).toEqual(observed);
    const session = await HbsSession.open({adapter, port: createDocumentPort(adapter, reopened.call),
      preservation: createDocumentPreservationPort(adapter, reopened.call), requestId: n => `restored-${n}`});
    expect(session.getSnapshot().content).toEqual(content);
    expect(session.getSnapshot().changes.length).toBeGreaterThan(0);
    expect((await reopened.call("read") as DocumentSnapshot<unknown>).baseline).toEqual(fixture);
    session.dispose(); await session.flushPreservation(); reopened.close();
  });

  it(`${adapter.kind}: stores the exact successful request and receipt atomically and rejects request id reuse`, async () => {
    const first = await createIndexedHbsAuthority(adapter, fixture);
    const request = {id: "retry-me", expectedVersion: 1, saved: changed(adapter, fixture), baseline: fixture};
    const original = await first.call("commit", request); first.close();
    const reopened = await createIndexedHbsAuthority(adapter, fixture);
    await reopened.call("commit", {...request, id: "later", expectedVersion: 2});
    expect(await reopened.call("commit", request)).toEqual(original);
    expect((await reopened.call("read") as DocumentSnapshot<unknown>).version).toBe(3);
    await expect(reopened.call("commit", {...request, saved: fixture})).rejects.toThrow("MODEL_REQUEST_ID_REUSED");
    reopened.close();
  });
}

it("resolves two competing tabs with one CAS receipt and one current HBS conflict", async () => {
  const {adapter, fixture} = cases[0]!;
  const first = await createIndexedHbsAuthority(adapter, fixture), second = await createIndexedHbsAuthority(adapter, fixture);
  const request = {id: "first", expectedVersion: 1, saved: "first", baseline: fixture};
  const replies = await Promise.all([first.call("commit", request), second.call("commit", {...request, id: "second", saved: "second"})]) as {type: string; snapshot: DocumentSnapshot<string>}[];
  expect(replies.map(value => value.type).sort()).toEqual(["conflict", "receipt"]);
  expect(replies[1]!.snapshot).toEqual(replies[0]!.snapshot);
  expect(await second.call("read")).toEqual(replies[0]!.snapshot);
  first.close(); second.close();
});

it("retries database contention caused by independent S′ without overwriting it", async () => {
  const {adapter, fixture} = cases[0]!;
  const first = await createIndexedHbsAuthority(adapter, fixture), second = await createIndexedHbsAuthority(adapter, fixture);
  const observed = decodeDocumentSnapshot(await first.call("read"), adapter), value = checkpoint(adapter, observed, "Preserved H");
  const replies = await Promise.all([first.call("savePreservation", value), second.call("commit", {id: "s", expectedVersion: 1, saved: "new", baseline: fixture})]);
  expect(replies[1]).toMatchObject({type: "receipt", snapshot: {version: 2}});
  expect(await first.call("readPreservation")).toEqual(value);
  expect(await first.call("read")).toEqual({version: 2, saved: "new", baseline: fixture});
  first.close(); second.close();
});

it("does not overwrite a concurrently replaced S′ slot", async () => {
  const {adapter, fixture} = cases[0]!;
  const first = await createIndexedHbsAuthority(adapter, fixture), second = await createIndexedHbsAuthority(adapter, fixture);
  const observed = decodeDocumentSnapshot(await first.call("read"), adapter);
  const one = checkpoint(adapter, observed, "one"), two = checkpoint(adapter, observed, "two");
  const result = await Promise.allSettled([first.call("savePreservation", one), second.call("savePreservation", two)]);
  expect(result[0]!.status).toBe("fulfilled");
  expect(result[1]).toMatchObject({status: "rejected", reason: {code: "MODEL_PRESERVATION_CONFLICT"}});
  expect(await first.call("readPreservation")).toEqual(one); first.close(); second.close();
});

it("does not acknowledge failed S/B or S′ writes or contaminate later reads", async () => {
  const {adapter, fixture} = cases[0]!;
  const authority = await createIndexedHbsAuthority(adapter, fixture);
  const before = decodeDocumentSnapshot(await authority.call("read"), adapter);
  const request = {id: "failure", expectedVersion: before.version, saved: "not durable", baseline: "not durable"};
  database.failNext(new PreviewStorageError("preview_storage_failed", "QuotaExceededError"));
  await expect(authority.call("commit", request)).rejects.toThrow("QuotaExceededError");
  expect(await authority.call("read")).toEqual(before);
  expect(await authority.call("readPreservation")).toBeNull();
  database.failNext(new PreviewStorageError("preview_storage_failed", "disk failure"));
  await expect(authority.call("savePreservation", checkpoint(adapter, before, "unsaved"))).rejects.toThrow("disk failure");
  expect(await authority.call("readPreservation")).toBeNull();
  expect(await authority.call("commit", request)).toMatchObject({type: "receipt", snapshot: {version: 2}});
  authority.close();
});

it("publishes a receipt only after the database transaction completes", async () => {
  const {adapter, fixture} = cases[0]!;
  const authority = await createIndexedHbsAuthority(adapter, fixture);
  let release!: () => void, completed = false;
  database.holdUntil(new Promise<void>(resolve => { release = resolve; }));
  const pending = authority.call("commit", {id: "held", expectedVersion: 1, saved: "durable", baseline: fixture}).then(reply => { completed = true; return reply; });
  await Promise.resolve(); await Promise.resolve();
  expect(completed).toBe(false);
  expect((await authority.call("read") as DocumentSnapshot<unknown>).version).toBe(1);
  release(); expect(await pending).toMatchObject({type: "receipt", snapshot: {version: 2}}); authority.close();
});

it("captures a structured request before awaiting storage so caller edits cannot change its payload", async () => {
  const {adapter, fixture} = cases[2]!;
  const authority = await createIndexedHbsAuthority(adapter, fixture);
  let release!: () => void;
  database.holdUntil(new Promise<void>(resolve => { release = resolve; }));
  const saved = {value: "original"};
  const pending = authority.call("commit", {id: "immutable", expectedVersion: 1, saved, baseline: fixture});
  saved.value = "later caller edit"; release();
  expect(await pending).toMatchObject({snapshot: {saved: {value: "original"}}});
  authority.close();
});

it("reset's clearPreservation removes only S′, keeps CAS history, and isolates all three kinds", async () => {
  const authorities = await Promise.all(cases.map(({adapter, fixture}) => createIndexedHbsAuthority(adapter, fixture)));
  for (let index = 0; index < authorities.length; index++) {
    const {adapter, fixture} = cases[index]!, authority = authorities[index]!;
    const observed = decodeDocumentSnapshot(await authority.call("read"), adapter);
    await authority.call("savePreservation", checkpoint(adapter, observed, changed(adapter, fixture)));
  }
  await authorities[0]!.call("clearPreservation");
  expect(await authorities[0]!.call("readPreservation")).toBeNull();
  expect(await authorities[1]!.call("readPreservation")).not.toBeNull();
  expect(await authorities[2]!.call("readPreservation")).not.toBeNull();
  expect(await authorities[0]!.call("read")).toEqual({version: 1, saved: textFixture, baseline: textFixture});
  for (const authority of authorities) authority.close();
});

it("allows explicit reset to remove a corrupt S′ without decoding or changing S/B", async () => {
  const {adapter, fixture} = cases[0]!;
  const authority = await createIndexedHbsAuthority(adapter, fixture);
  const original = await authority.call("read");
  const db = await database.open(`${HBS_ADAPTER_DATABASE_PREFIX}text`);
  for (const raw of ["not JSON", JSON.stringify({revision: 1, checkpoint: {format: "broken"}})]) {
    const stored = await db.load();
    await db.commit(stored.revision, new Map([["document:text:S′", raw]]), false);
    await expect(authority.call("readPreservation")).rejects.toThrow();
    await authority.call("clearPreservation");
    expect(await authority.call("readPreservation")).toBeNull();
    expect(await authority.call("read")).toEqual(original);
  }
  db.close(); authority.close();
});

it("does not let reset's clear overwrite a concurrently replaced S′", async () => {
  const {adapter, fixture} = cases[0]!;
  const first = await createIndexedHbsAuthority(adapter, fixture), second = await createIndexedHbsAuthority(adapter, fixture);
  const observed = decodeDocumentSnapshot(await first.call("read"), adapter);
  const initial = checkpoint(adapter, observed, "initial"), replacement = checkpoint(adapter, observed, "replacement");
  await first.call("savePreservation", initial);
  const result = await Promise.allSettled([first.call("savePreservation", replacement), second.call("clearPreservation")]);
  expect(result[0]!.status).toBe("fulfilled");
  expect(result[1]).toMatchObject({status: "rejected", reason: {code: "MODEL_PRESERVATION_CONFLICT"}});
  expect(await second.call("readPreservation")).toEqual(replacement); first.close(); second.close();
});

it("inspects actual IndexedDB entry keys and unchanged persisted value strings", async () => {
  const {adapter, fixture} = cases[0]!;
  const authority = await createIndexedHbsAuthority(adapter, fixture);
  const observed = decodeDocumentSnapshot(await authority.call("read"), adapter);
  await authority.call("savePreservation", checkpoint(adapter, observed, "My H"));
  const db = await database.open(`${HBS_ADAPTER_DATABASE_PREFIX}text`), stored = await db.load();
  const inspected = await authority.call("inspectStorage") as {snapshot: unknown; storage: Record<string, {table: string; columns: string[]; rows: {key: string; value: string}[]}[]>};
  expect(inspected.snapshot).toEqual(observed);
  for (const tables of Object.values(inspected.storage)) for (const table of tables) {
    expect(table.table).toContain("IndexedDB"); expect(table.columns).toEqual(["key", "value"]);
    for (const row of table.rows) expect(row.value).toBe(stored.entries.get(row.key));
  }
  expect(inspected.storage.S![0]!.rows.map(row => row.key)).toEqual(["document:text:version", "document:text:S"]);
  expect(inspected.storage["S′"]![0]!.rows[0]!.key).toBe("document:text:S′"); db.close(); authority.close();
});

it("rejects unavailable or corrupt storage without reseeding or a localStorage fallback", async () => {
  const {adapter, fixture} = cases[0]!;
  const authority = await createIndexedHbsAuthority(adapter, fixture); authority.close();
  const db = await database.open(`${HBS_ADAPTER_DATABASE_PREFIX}text`), stored = await db.load();
  await db.commit(stored.revision, new Map([["document:text:S", "not JSON"]]), false);
  await expect(createIndexedHbsAuthority(adapter, fixture)).rejects.toThrow("MODEL_STORAGE_CORRUPT");
  expect((await db.load()).entries.get("document:text:S")).toBe("not JSON"); db.close();
  configurePreviewDatabase(async () => { throw new PreviewStorageError("preview_storage_unavailable", "disabled"); });
  await expect(createIndexedHbsAuthority(adapter, fixture)).rejects.toThrow("preview_storage_unavailable");
});

it("validates checkpoint identity and missing save payload before changing S′", async () => {
  const {adapter, fixture} = cases[1]!;
  const authority = await createIndexedHbsAuthority(adapter, fixture);
  const observed = decodeDocumentSnapshot(await authority.call("read"), adapter);
  const prior = checkpoint(adapter, observed, fixture);
  await authority.call("savePreservation", prior);
  await expect(authority.call("savePreservation")).rejects.toThrow("MODEL_RECOVERY_INVALID");
  await expect(authority.call("savePreservation", {...prior, content: {...noteFixture, pageId: "another-page"}})).rejects.toThrow("MODEL_INVALID_DOCUMENT");
  expect(await authority.call("readPreservation")).toEqual(prior); authority.close();
});

it("permits concurrent sessions to initialize the same empty S′ without a false preservation conflict", async () => {
  const {adapter, fixture} = cases[0]!;
  const authority = await createIndexedHbsAuthority(adapter, fixture);
  const sessions = await Promise.all([1, 2].map(index => HbsSession.open({adapter,
    port: createDocumentPort(adapter, authority.call), preservation: createDocumentPreservationPort(adapter, authority.call), requestId: n => `${index}-${n}`})));
  expect(sessions.map(session => session.getSnapshot().preservationError)).toEqual([undefined, undefined]);
  expect(database.metrics.commits).toBe(2); // bootstrap and one durable checkpoint
  for (const session of sessions) {session.dispose(); await session.flushPreservation();} authority.close();
});

it("reuses only a fully equal normalized checkpoint, including review provenance and observed S/B", async () => {
  const {adapter, fixture} = cases[0]!;
  const authority = await createIndexedHbsAuthority(adapter, fixture);
  const observed = decodeDocumentSnapshot(await authority.call("read"), adapter), value = checkpoint(adapter, observed, "candidate");
  expect(await authority.call("savePreservation", value)).toEqual({revision: 1});
  const commits = database.metrics.commits;
  const reordered = Object.fromEntries(Object.entries(value).reverse());
  expect(await authority.call("savePreservation", reordered)).toEqual({revision: 1});
  expect(database.metrics.commits).toBe(commits);
  expect(await authority.call("savePreservation", {...value, reviewAncestor: "origin"})).toEqual({revision: 2});
  expect(await authority.call("savePreservation", {...value, observed: {...observed, version: 2}})).toEqual({revision: 3});
  authority.close();
});
