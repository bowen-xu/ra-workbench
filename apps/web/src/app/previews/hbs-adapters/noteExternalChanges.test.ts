import { describe, expect, it } from "vitest";
import { createNoteDocumentAdapter } from "@research-assistant/core/hbs";
import { noteBlockId, type NoteBlockV1 } from "@research-assistant/core/notes";
import { blocksToEditor, editorToBlocks } from "@research-assistant/core/notes";
import { changeNoteExample, noteExternalActions } from "./noteExternalChanges";
import { noteFixture } from "./fixtures";
import { directTextDiff } from "../../../core/components/hbs/codeMirrorHbsPolicy";
const adapter=createNoteDocumentAdapter(directTextDiff);
const assets={image:{id:"a".repeat(64),displayName:"image.png",mimeType:"image/png"},file:{id:"b".repeat(64),displayName:"file.txt",mimeType:"text/plain"}};
describe("external note inputs use real document structure and codecs",()=>{
  for(const action of noteExternalActions.filter(action=>action.id!=="childPage"))it(action.label,()=>{
    const next=changeNoteExample(noteFixture,action.id,50,assets);
    expect(adapter.equal(next,noteFixture)).toBe(false);
    expect(adapter.diff(noteFixture,next).length).toBeGreaterThan(0);
    const portable=blocksToEditor(next.blocks);
    expect(portable.length).toBe(next.blocks.length);
    // The same codec as the production editor must accept every scenario.
    expect(()=>editorToBlocks(portable)).not.toThrow();
    expect(adapter.equal(noteFixture,structuredClone(noteFixture))).toBe(true);
  });
  it("reuses stable block identity for repeated edits instead of adding duplicates",()=>{
    const first=changeNoteExample(noteFixture,"codeBlock",1);
    const next=changeNoteExample(first,"codeBlock",2);
    expect(next.blocks.length).toBe(first.blocks.length);
    expect(next.blocks.filter(block=>block.id==="hbs-demo-codeBlock")).toHaveLength(1);
  });
  it("preserves real child targets and reports a missing lifecycle dependency",()=>{
    expect(()=>changeNoteExample(noteFixture,"childPage",1)).toThrow("不会伪造父子关系");
    const block:NoteBlockV1={id:noteBlockId("existing-child"),type:"childPage",props:{},children:[],content:{kind:"reference",targetKind:"note_page",targetId:"actual-child",labelSnapshot:"before",quoteSnapshot:""}};
    const document={...noteFixture,blocks:[...noteFixture.blocks,block]};
    const next=changeNoteExample(document,"childPage",2).blocks.at(-1)!;
    expect(next.content).toMatchObject({targetId:"actual-child",labelSnapshot:"外部修改 v2"});
  });
  it("reports unavailable geometry and assets instead of fabricating correspondences",()=>{
    expect(()=>changeNoteExample(noteFixture,"image",1)).toThrow("真实附件尚未导入");
    let document=changeNoteExample(noteFixture,"tableRowDelete",1);
    document=changeNoteExample(document,"tableRowDelete",2);
    expect(()=>changeNoteExample(document,"tableRowDelete",3)).toThrow("至少保留一行");
  });
});
