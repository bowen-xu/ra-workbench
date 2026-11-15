// @vitest-environment happy-dom
import {BlockNoteEditor} from "@blocknote/core";
import {expect,it} from "vitest";
import {baseBlockEditorSchema} from "./baseSchema";
import {blockEditorDefaults} from "./defaults";
import {blockNoteToNoteDocument,noteDocumentToBlockNote} from "../../features/notes/editor/blockNoteAdapter";
import {decodeNoteDocument,notePageId} from "@research-assistant/core/notes";

function editor(){return BlockNoteEditor.create({...blockEditorDefaults,schema:baseBlockEditorSchema,initialContent:[{
  id:"table",type:"table",props:{caption:"results",textAlignment:"center"},content:{type:"tableContent",rows:[
    {cells:["name","value"]},{cells:["sample","42"]},
  ]},
}]});}

it.each([{headerRows:1},{headerCols:1},{headerRows:1,headerCols:1}])("round-trips table headers %j through the actual editor, domain decoder and HTML",async headers=>{
  const first=editor(),table=first.getBlock("table")!;
  first.mount(document.createElement("div"));
  if(table.type!=="table")throw new Error("table fixture missing");
  first.updateBlock(table,{content:{...table.content,...headers}});
  const saved=decodeNoteDocument(blockNoteToNoteDocument(notePageId("table-headers"),first.document));
  expect(saved.blocks[0]?.content).toMatchObject(headers);
  const reopened=BlockNoteEditor.create({...blockEditorDefaults,schema:baseBlockEditorSchema,initialContent:noteDocumentToBlockNote(saved)});
  expect(reopened.getBlock("table")?.content).toMatchObject(headers);
  expect(reopened.getBlock("table")?.props).toMatchObject({caption:"results",textAlignment:"center"});
  const html=reopened.blocksToFullHTML(),container=document.createElement("div");container.innerHTML=html;
  expect(container.querySelectorAll("th")).toHaveLength(headers.headerRows && headers.headerCols ? 3 : 2);
  const parsed=await reopened.tryParseHTMLToBlocks(html);
  expect(parsed[0]?.content).toMatchObject(headers);
  expect(reopened.undo()).toBe(false);
  expect(first.undo()).toBe(true);
  expect((first.getBlock("table")?.content as {headerRows?:number;headerCols?:number}).headerRows).toBeUndefined();
  expect((first.getBlock("table")?.content as {headerRows?:number;headerCols?:number}).headerCols).toBeUndefined();
  expect(first.redo()).toBe(true);expect(first.getBlock("table")?.content).toMatchObject(headers);
  first.unmount();
});

it("rejects invalid header counts and normalizes disabled headers without changing older tables",()=>{
  const plain=blockNoteToNoteDocument(notePageId("table-headers"),editor().document),block=plain.blocks[0]!;
  const withHeaders=(headers:object)=>({...plain,blocks:[{...block,content:{...block.content,...headers}}]});
  expect(decodeNoteDocument(withHeaders({headerRows:0,headerCols:0}))).toEqual(plain);
  for(const value of [-1,1.5,"1",10_001])expect(()=>decodeNoteDocument(withHeaders({headerRows:value}))).toThrow("表格表头数量无效");
});
