// @vitest-environment happy-dom
import {beforeEach,expect,it,vi} from "vitest";
import {NotesClient} from "./NotesClient";
import {browserNotesGateway} from "./gateways/browserNotesGateway";
import {selectAppStorage} from "../../runtime/appStorage";
import {noteResourceAccess,noteAnnotationAccess,literatureNoteAccess} from "./noteResourceAccess";

beforeEach(()=>{localStorage.clear();selectAppStorage(localStorage);});

it("adapts one provider without exposing its private implementation or unrelated storage operations",()=>{
  const client=new NotesClient(browserNotesGateway);
  const resources=noteResourceAccess(client);
  expect(noteResourceAccess(client)).toBe(resources);
  expect(noteAnnotationAccess(client)).toBe(noteAnnotationAccess(client));
  expect(literatureNoteAccess(client)).toBe(literatureNoteAccess(client));
  expect(noteResourceAccess(new NotesClient(browserNotesGateway))).not.toBe(resources);
  expect(resources).not.toBeInstanceOf(NotesClient);
  expect(Object.keys(resources.gateway)).not.toContain("createNoteAsset");
  expect(Object.keys(resources.gateway)).not.toContain("rebuildNoteIndex");
  expect(Object.keys(resources.creation)).toEqual(["create","isDraft","forgetEmpty"]);
  expect(Object.keys(resources.history)).toEqual(["closeGroups","record","run"]);
  expect(Object.keys(noteAnnotationAccess(client).gateway)).toEqual([
    "listReadingNotes","getReadingNote","createReadingNote","updateReadingNote","setReadingNoteStatus",
  ]);
});

it("mutates the existing storage and publishes through the original resource observer exactly once",async()=>{
  const client=new NotesClient(browserNotesGateway),resources=noteResourceAccess(client),annotations=noteAnnotationAccess(client);
  const changed=vi.fn(),stop=annotations.subscribe(changed);
  try{
    const before=client.getVersion();
    const created=await resources.mutate(gateway=>gateway.createPage({title:"同一存储"}));
    expect((await browserNotesGateway.loadDocument(created.page.id)).page.title).toBe("同一存储");
    expect(changed).toHaveBeenCalledOnce();
    expect(resources.getVersion()).toBe(before+1);
    expect(annotations.getChangesSince(before)).toEqual([{kind:"all"}]);
  }finally{stop();}
});

it("retains failure identity and does not publish an uncommitted mutation",async()=>{
  const client=new NotesClient(browserNotesGateway),resources=noteResourceAccess(client),changed=vi.fn();
  const stop=resources.subscribe(changed),failure=new Error("original_store_failure");
  try{
    await expect(resources.mutate(async()=>{throw failure;})).rejects.toBe(failure);
    expect(changed).not.toHaveBeenCalled();
    expect(client.getVersion()).toBe(0);
  }finally{stop();}
});
