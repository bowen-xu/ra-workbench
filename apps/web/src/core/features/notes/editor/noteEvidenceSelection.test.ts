import { Schema } from "@tiptap/pm/model";
import { expect, it } from "vitest";
import { parseSourceLink } from "@research-assistant/core/source";
import { noteEvidenceSelection } from "./noteEvidenceSelection";
const schema = new Schema({nodes:{doc:{content:"blockContainer+"},blockContainer:{attrs:{id:{default:""}},content:"paragraph blockContainer*"},paragraph:{content:"(text|hardBreak)*"},text:{},hardBreak:{inline:true,group:"inline"}}});
it("captures block-relative Unicode coordinates and separate quotes across blocks", () => {
  const p = (text:string) => schema.node("paragraph",null,schema.text(text));
  const block = (id:string,text:string,children:readonly import("@tiptap/pm/model").Node[]=[]) => schema.node("blockContainer",{id},[p(text),...children]);
  const doc = schema.node("doc",null,[block("one","🧪AB",[block("child","EF")]),block("two","CD")]);
  const selections = noteEvidenceSelection(doc,"page",2,doc.content.size);
  expect(selections.map(item=>item.text)).toEqual(["🧪AB","EF","CD"]);
  expect(parseSourceLink(selections[0]!.source).locator).toEqual({kind:"block",blockId:"one",range:{startLine:1,startColumn:1,endLine:1,endColumn:3}});
  const partial = noteEvidenceSelection(doc,"page",4,6);
  expect(partial[0]!.text).toBe("AB");
  expect(parseSourceLink(partial[0]!.source).locator).toEqual({kind:"block",blockId:"one",range:{startLine:1,startColumn:2,endLine:1,endColumn:3}});
});
