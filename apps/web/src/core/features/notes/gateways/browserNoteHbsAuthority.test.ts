// @vitest-environment happy-dom
import {beforeEach, expect, it, vi} from "vitest";
import {createNoteDocumentFromText, noteBlockId, noteDocumentPlainText} from "@research-assistant/core/notes";
import {createDocumentPort, createDocumentPreservationPort} from "@research-assistant/data/hbs";
import {createNoteDocumentAdapter, HbsSession, type DocumentCommit} from "@research-assistant/core/hbs";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {selectAppStorage} from "../../../runtime/appStorage";
import {createTestNamespacedStorage} from "../../../runtime/testing/createTestNamespacedStorage";
import {createMemoryNotesDatabase} from "../../../runtime/testing/memoryNotesDatabase";
import {configureBrowserNotesDatabase} from "./browserNotesDatabase";
import {browserNotesGateway as notes} from "./browserNotesGateway";
import {readState} from "./browserNotesStorage";
import {createBrowserNoteHbsAuthority} from "./browserNoteHbsAuthority";

beforeEach(() => { localStorage.clear(); selectAppStorage(localStorage); configureBrowserNotesDatabase(createMemoryNotesDatabase().open); });
async function fixture() {
  const page = await notes.createPage({title: "Synthetic HBS note"}), id = page.page.id;
  const doc = (text: string) => ({...createNoteDocumentFromText(id, text, () => noteBlockId("content")), title: "Synthetic HBS note"});
  const original = doc("80%"), authority = createBrowserNoteHbsAuthority();
  await authority.saveExternal({document: original, expectedRevision: 0, saveReason: "structure"}, original);
  return {id, doc, original, authority};
}
it("uses the existing note repository and commits S/B/dedup in one transaction", async () => {
  const {id, doc, original, authority} = await fixture();
  const request = {id: "same-exact-request", expectedVersion: 1, saved: doc("85%"), baseline: original};
  const result = await authority.commit(id, request);
  expect(result).toMatchObject({type: "receipt", snapshot: {version: 2, saved: doc("85%"), baseline: original}});
  expect((await notes.loadDocument(id)).document).toEqual(doc("85%"));
  const revisionCount = (await readState()).revisions.length;
  expect(await createBrowserNoteHbsAuthority().commit(id, request)).toEqual(result);
  expect((await readState()).revisions).toHaveLength(revisionCount);
  await expect(authority.commit(id, {...request, saved: doc("different")})).rejects.toMatchObject({code: "MODEL_INVALID_REQUEST"});
  const stale = await authority.commit(id, {...request, id: "stale"});
  expect(stale).toMatchObject({type: "conflict", snapshot: {version: 2}});
});
it("advances a baseline-only revision without rewriting identical body or its history", async () => {
  const {id, doc, authority} = await fixture(), before = await readState();
  await authority.commit(id, {id: "baseline-only", expectedVersion: 1, saved: doc("80%"), baseline: doc("80")});
  expect((await notes.loadDocument(id)).page.revision).toBe(2);
  expect((await readState()).revisions).toEqual(before.revisions);
});
it("a failed transaction leaves S/B and receipt unchanged; original request can be retried", async () => {
  const memory = createMemoryNotesDatabase(); let fail = false;
  configureBrowserNotesDatabase(async name => { const db = await memory.open(name); return {...db, commit: async (...args) => {
    if (fail) throw new Error("disk full"); return db.commit(...args);
  }}; });
  const {id, doc, original, authority} = await fixture();
  const request = {id: "retry-after-rollback", expectedVersion: 1, saved: doc("85%"), baseline: original};
  fail = true; await expect(authority.commit(id, request)).rejects.toThrow("disk full");
  expect(await authority.read(id)).toEqual({version: 1, saved: original, baseline: original});
  fail = false; expect(await authority.commit(id, request)).toMatchObject({type: "receipt", snapshot: {version: 2}});
});
it("atomically saves a title/body join, retries failure and reopens with its original revision", async () => {
  const memory = createMemoryNotesDatabase(); let fail = false;
  configureBrowserNotesDatabase(async name => {const db = await memory.open(name); return {...db, commit: async (...args) => {
    if (fail) throw new Error("disk full"); return db.commit(...args);
  }};});
  const {id, original, authority} = await fixture();
  const joined = {...original, title: "Synthetic HBS note80%", blocks: []};
  const request = {id: "title-and-body", expectedVersion: 1, saved: joined, baseline: joined};
  fail = true; await expect(authority.commit(id, request)).rejects.toThrow("disk full");
  expect(await authority.read(id)).toEqual({version: 1, saved: original, baseline: original});
  expect((await notes.loadDocument(id)).page.title).toBe(original.title);
  fail = false; const receipt = await authority.commit(id, request);
  expect(await authority.commit(id, request)).toEqual(receipt);
  selectAppStorage(createTestNamespacedStorage(localStorage, ""));
  expect((await createBrowserNoteHbsAuthority().read(id)).saved).toEqual(joined);
  const versions = await notes.listRevisions(id);
  const prior = await notes.loadRevision(id, versions.find(version => version.pageRevision === 1)!.id);
  expect(prior).toEqual(original);
});
it("imports the accepted legacy title rather than accepting an externally renamed candidate", async () => {
  const {id, original} = await fixture();
  const state = await readState();
  // A legacy baseline has a body but no title field.
  const {title: _title, ...body} = original;
  const {writeState} = await import("./browserNotesStorage");
  await writeState({...state, pages: state.pages.map(page => page.id === id ? {...page, title: "Candidate title"} : page),
    documents: {...state.documents, [id]: body},
    hbsRecords: {[id]: {baseline: body}}});
  const authority = createBrowserNoteHbsAuthority({legacyBaseline: () => original});
  expect((await authority.read(id)).saved.title).toBe("Candidate title");
  expect((await authority.read(id)).baseline.title).toBe(original.title);
});
it("reopens raw empty title text while navigation uses its default label", async () => {
  const {id,original,authority} = await fixture();
  const empty = {...original,title:""};
  await authority.commit(id,{id:"empty-title",expectedVersion:1,saved:empty,baseline:empty});
  selectAppStorage(createTestNamespacedStorage(localStorage,""));
  expect((await createBrowserNoteHbsAuthority().read(id)).saved.title).toBe("");
  const loaded = await notes.loadDocument(id);
  expect(loaded.document.title).toBe(""); expect(loaded.page.title).toBe("无标题");
});
it("S′ survives reopening and saving candidate edits never accepts them", async () => {
  const {id, doc, original, authority} = await fixture();
  await authority.saveExternal({document: doc("85%"), expectedRevision: 1, saveReason: "structure"}, original);
  const adapter = createNoteDocumentAdapter(directTextDiff);
  let serial = 0;
  const open = (port = authority) => HbsSession.open({adapter, requestId: () => `session:${++serial}`,
    port: createDocumentPort(adapter, (op, value) => op === "read" ? port.read(id) : port.commit(id, value as DocumentCommit<unknown>)),
    preservation: createDocumentPreservationPort(adapter, (op, value) => op === "readPreservation" ? port.readPreservation(id) : port.savePreservation(id, value))});
  const session = await open(); session.edit(doc("185%")); await session.save();
  expect(await authority.read(id)).toMatchObject({saved: doc("185%"), baseline: original});
  session.dispose();
  // A distinct storage owner models a process/window reopening the same DB.
  selectAppStorage(createTestNamespacedStorage(localStorage, ""));
  const reopenedAuthority = createBrowserNoteHbsAuthority(), reopened = await open(reopenedAuthority);
  expect(noteDocumentPlainText(reopened.getSnapshot().content)).toBe("185%");
  expect(reopened.getSnapshot().changes).toHaveLength(1);
  await reopenedAuthority.saveExternal({document: doc("90%"), expectedRevision: 3, saveReason: "structure"}, original);
  await reopened.syncLatest();
  expect(noteDocumentPlainText(reopened.getSnapshot().content)).toBe("185%");
  expect(reopened.getSnapshot().conflicts).toHaveLength(1); reopened.dispose();
});
it("does not let an old authority or snapshot write into a new runtime owner", async () => {
  const {id, doc, authority} = await fixture(), old = await readState();
  selectAppStorage(createTestNamespacedStorage(localStorage, "another-runtime:"));
  await expect(authority.commit(id, {id: "old-runtime", expectedVersion: 1, saved: doc("wrong"), baseline: doc("wrong")})).rejects.toMatchObject({code: "MODEL_OWNER_CHANGED"});
  const {writeState} = await import("./browserNotesStorage");
  await expect(writeState(old)).rejects.toMatchObject({code: "operation_cancelled"});
  expect((await readState()).pages).toEqual([]);
});
it("imports an existing legacy baseline and validates recovery page identity", async () => {
  const {id, doc, original, authority} = await fixture();
  const acknowledge = vi.fn();
  // Existing persisted B remains authoritative over a later legacy callback.
  const migrated = createBrowserNoteHbsAuthority({legacyBaseline: () => doc("ignored"), acknowledge});
  expect((await migrated.read(id)).baseline).toEqual(original);
  const checkpoint = {format: 1, kind: "note", observed: await authority.read(id), content: doc("draft"), ancestor: original, baselineAncestor: original, ordinary: []};
  await migrated.savePreservation(id, checkpoint);
  await expect(migrated.savePreservation(id, {...checkpoint, content: {...original, pageId: "other"}})).rejects.toMatchObject({code: "MODEL_INVALID_DOCUMENT"});
  expect(await migrated.readPreservation(id)).toEqual(checkpoint);
  await migrated.commit(id, {id: "ack", expectedVersion: 1, saved: doc("85%"), baseline: original}); expect(acknowledge).toHaveBeenCalledOnce();
});
it("reads only the opened document and checkpoints without rewriting unrelated navigation or history",async()=>{
  const memory=createMemoryNotesDatabase(),queries:unknown[]=[];
  configureBrowserNotesDatabase(async name=>{const db=await memory.open(name);return{...db,read:(query,select)=>{queries.push(query);return db.read(query,select);}};});
  const {id,doc,original,authority}=await fixture();
  const other=await notes.createPage({title:"unrelated"});
  await notes.updatePage({pageId:other.page.id,expectedRevision:other.page.revision,isFavorite:true});
  const before=await readState();queries.length=0;memory.metrics.documentReads=0;
  const observed=await authority.read(id);expect(await authority.readPreservation(id)).toBeNull();
  await authority.savePreservation(id,{format:1,kind:"note",observed,content:doc("draft"),ancestor:original,baselineAncestor:original,ordinary:[]});
  expect(memory.metrics.documentReads).toBe(3);
  expect(queries).toEqual(Array.from({length:3},()=>({pages:[id],documents:[id],operations:[`$hbs:${id}`]})));
  const after=await readState();expect(after.favoriteOrder).toEqual(before.favoriteOrder);expect(after.pages).toEqual(before.pages);
  expect(after.documents[other.page.id]).toEqual(other.document);expect(after.revisions).toEqual(before.revisions);
  queries.length=0;memory.metrics.documentReads=0;
  await authority.commit(id,{id:"focused-commit",expectedVersion:1,saved:doc("edited"),baseline:original});
  expect(memory.metrics.documentReads).toBe(1);expect((await readState()).documents[other.page.id]).toEqual(other.document);
});
