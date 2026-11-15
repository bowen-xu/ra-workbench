// @vitest-environment happy-dom
import {beforeEach, expect, it} from "vitest";
import {createNoteDocumentFromText, decodeNoteDocument, noteBlockId, noteDocumentPlainText, notePageId} from "@research-assistant/core/notes";
import {selectAppStorage} from "../../../runtime/appStorage";
import {createMemoryNotesDatabase} from "../../../runtime/testing/memoryNotesDatabase";
import {configureBrowserNotesDatabase, type NoteRead} from "./browserNotesDatabase";
import {readState, writeState} from "./browserNotesStorage";
import {browserNotesGateway as notes} from "./browserNotesGateway";

beforeEach(()=>{localStorage.clear();selectAppStorage(localStorage);});
it("reads selected revisions and canonical rich documents in one keyed transaction, without unrelated records",async()=>{
  const memory=createMemoryNotesDatabase(), queries: NoteRead[]=[];
  configureBrowserNotesDatabase(async name=>{const db=await memory.open(name);return {...db,read:async(...args)=>{queries.push(args[0]);return db.read(...args);}};});
  const first=await notes.createPage({title:"first"}), other=await notes.createPage({title:"unrelated"});
  const rich=decodeNoteDocument({schemaVersion:1,pageId:first.page.id,blocks:[
    {id:"parent",type:"paragraph",props:{},content:{kind:"inline",items:[{type:"text",text:"Text",styles:[]},{type:"inlineFormula",source:"E=mc^2"}]},children:[
      {id:"child",type:"readingNoteReference",props:{},content:{kind:"reference",targetKind:"reading_note",targetId:"reading",labelSnapshot:"Source",quoteSnapshot:"Quote"},children:[]},
    ]},
    {id:"table",type:"table",props:{},content:{kind:"table",rows:[[{kind:"inline",items:[{type:"text",text:"Cell",styles:[]}]}]]},children:[]},
  ]});
  const state=await readState();
  await writeState({...state,pages:state.pages.map(page=>page.id===first.page.id?{...page,revision:7}:page),documents:{...state.documents,[first.page.id]:rich,[other.page.id]:{schemaVersion:99} as never}});
  queries.length=0;memory.metrics.documentReads=0;memory.metrics.revisionReads=0;
  const result=await notes.loadIndexDocuments([first.page.id,first.page.id]);
  expect(result).toEqual([{pageId:first.page.id,revision:7,document:rich}]);
  expect(noteDocumentPlainText(result[0]!.document)).toBe("TextE=mc^2\nSource Quote\nCell");
  expect(queries).toEqual([{pages:[first.page.id],documents:[first.page.id]}]);
  expect(memory.metrics.documentReads).toBe(1);expect(memory.metrics.revisionReads).toBe(0);
});
it("rejects missing selected bodies without writing and accepts exactly twenty keys",async()=>{
  const memory=createMemoryNotesDatabase();configureBrowserNotesDatabase(memory.open);
  const state=await readState();
  const pages=Array.from({length:20},(_,i)=>({...{title:"Synthetic",position:i,status:"active" as const,isFavorite:false,revision:0,schemaVersion:1,projectIds:[],tagIds:[],createdAt:"now",updatedAt:"now"},id:notePageId(`page-${i}`)}));
  const documents=Object.fromEntries(pages.map(page=>[page.id,createNoteDocumentFromText(page.id,"Body",()=>noteBlockId(`block-${page.id}`))]));
  await writeState({...state,pages,documents});
  const before=memory.metrics.commits;
  expect(await notes.loadIndexDocuments(pages.map(page=>page.id))).toHaveLength(20);
  await expect(notes.loadIndexDocuments([pages[0]!.id,notePageId("missing")])).rejects.toMatchObject({code:"not_found"});
  expect(memory.metrics.commits).toBe(before);
});
it("checks empty and over-budget requests before opening storage",async()=>{
  configureBrowserNotesDatabase(async()=>{throw Error("unexpected storage access");});
  expect(await notes.loadIndexDocuments([])).toEqual([]);
  await expect(notes.loadIndexDocuments(Array.from({length:21},()=>notePageId("id")))).rejects.toMatchObject({code:"invalid_input",message:expect.stringContaining("notes_index_documents_limit")});
});
