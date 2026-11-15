// @vitest-environment happy-dom
import {beforeEach, expect, it} from "vitest";
import {noteBlockId, noteDocumentPlainText} from "@research-assistant/core/notes";
import {NotesGatewayError} from "@research-assistant/data/notes";
import {selectAppStorage} from "../../../runtime/appStorage";
import {createMemoryNotesDatabase} from "../../../runtime/testing/memoryNotesDatabase";
import {configureBrowserNotesDatabase} from "./browserNotesDatabase";
import {readState, writeState} from "./browserNotesStorage";
import {browserNotesGateway as notes} from "./browserNotesGateway";

beforeEach(() => {localStorage.clear(); selectAppStorage(localStorage);});

async function fixture() {
  const source = await notes.createPage({title:"source"});
  const target = await notes.createPage({title:"target"});
  const other = await notes.createPage({title:"unrelated"});
  const orphan = await notes.upsertLink({sourcePageId:source.page.id,target:{kind:"note_page",id:target.page.id,labelSnapshot:"target"},relation:"reference"});
  const unrelated = await notes.upsertLink({sourcePageId:source.page.id,target:{kind:"note_page",id:other.page.id,labelSnapshot:"other"},relation:"reference"});
  // A stale imported relationship whose source has gone away. Normal deletion
  // already removes its outgoing links; the read path also repairs old rows.
  const state = await readState();
  const documents = {...state.documents}; delete documents[source.page.id];
  await writeState({...state,pages:state.pages.filter(page=>page.id!==source.page.id),documents});
  return {source,target,other,orphan,unrelated};
}

it("removes only missing-source backlinks durably without loading bodies", async () => {
  const memory = createMemoryNotesDatabase(); configureBrowserNotesDatabase(memory.open);
  const {target,other,orphan,unrelated} = await fixture();
  memory.metrics.documentReads=0; memory.metrics.revisionReads=0;
  expect(await notes.listBacklinks("note_page",target.page.id)).toEqual([]);
  expect(memory.metrics.documentReads).toBe(0); expect(memory.metrics.revisionReads).toBe(0);
  const remaining = (await readState({metadataOnly:true})).links;
  expect(remaining.some(link=>link.id===orphan.id)).toBe(false);
  expect(remaining.some(link=>link.id===unrelated.id)).toBe(true);
  const commits = memory.metrics.commits;
  expect(await notes.listBacklinks("note_page",target.page.id)).toEqual([]);
  expect(memory.metrics.commits).toBe(commits);
  expect(await notes.listBacklinks("note_page",other.page.id)).toEqual([]);
});

it("keeps archived and trashed sources and preserves outgoing broken references and body text", async () => {
  const memory = createMemoryNotesDatabase(); configureBrowserNotesDatabase(memory.open);
  const source = await notes.createPage({title:"source"}), target = await notes.createPage({title:"target"});
  const saved = await notes.saveDocument({document:{...source.document,blocks:[{
    id:noteBlockId("mention"),type:"paragraph",props:{},children:[],content:{kind:"inline",items:[
      {type:"mention",targetKind:"note_page",targetId:target.page.id,labelSnapshot:"Visible quotation"},
    ]},
  }]},expectedRevision:source.page.revision,saveReason:"manual"});
  const archived = await notes.setPageStatus({id:source.page.id,status:"archived",expectedRevision:saved.page.revision});
  expect(await notes.listBacklinks("note_page",target.page.id)).toHaveLength(1);
  await notes.setPageStatus({id:source.page.id,status:"trashed",expectedRevision:archived.revision});
  expect(await notes.listBacklinks("note_page",target.page.id)).toHaveLength(1);
  const targetTrash = await notes.setPageStatus({id:target.page.id,status:"trashed",expectedRevision:target.page.revision});
  await notes.permanentlyDeletePage({id:target.page.id,expectedRevision:targetTrash.revision});
  expect(await notes.listOutgoing(source.page.id)).toEqual([expect.objectContaining({isBroken:true})]);
  // The destination's absence does not invalidate a backlink's existing source.
  expect(await notes.listBacklinks("note_page",target.page.id)).toHaveLength(1);
  const after = await notes.loadDocument(source.page.id);
  expect(after.document).toEqual(saved.document);
  expect(noteDocumentPlainText(after.document)).toBe("Visible quotation");
});

it("retains rows when a read or cleanup write fails, then permits retry", async () => {
  const memory = createMemoryNotesDatabase();
  let failure: "read" | "write" | undefined;
  configureBrowserNotesDatabase(async name => {
    const db = await memory.open(name);
    return {...db,read:async (...args) => {
      if (failure === "read") throw new NotesGatewayError("storage_error","TEST_READ_FAILED");
      return db.read(...args);
    },commit:async (...args) => {
      if (failure === "write") throw new NotesGatewayError("storage_error","TEST_WRITE_FAILED");
      return db.commit(...args);
    }};
  });
  const {target,orphan} = await fixture();
  for (const reason of ["read","write"] as const) {
    failure=reason;
    await expect(notes.listBacklinks("note_page",target.page.id)).rejects.toMatchObject({code:"storage_error"});
    failure=undefined;
    expect((await readState({metadataOnly:true})).links.some(link=>link.id===orphan.id)).toBe(true);
  }
  expect(await notes.listBacklinks("note_page",target.page.id)).toEqual([]);
});

it("does not prune a source restored by another writer after the metadata read", async () => {
  const memory = createMemoryNotesDatabase();
  let restore: (()=>Promise<void>) | undefined;
  configureBrowserNotesDatabase(async name => {
    const db = await memory.open(name);
    return {...db,commit:async (version,changes) => {
      const action = restore;
      if (action && changes.some(change=>change.store==="links" && change.value===undefined)) {
        restore=undefined; await action();
      }
      return db.commit(version,changes);
    }};
  });
  const {source,target,orphan} = await fixture();
  const token = localStorage.getItem("research-assistant.notes.v1.indexeddb")!;
  const db = await memory.open("research-assistant.notes."+token);
  restore = async () => {
    const current = await db.read({pages:[]});
    await db.commit(current.version,[{store:"pages",key:source.page.id,value:source.page}]);
  };
  await expect(notes.listBacklinks("note_page",target.page.id)).rejects.toMatchObject({code:"revision_conflict"});
  expect(await notes.listBacklinks("note_page",target.page.id)).toEqual([expect.objectContaining({id:orphan.id})]);
});
