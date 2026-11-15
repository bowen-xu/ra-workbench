// @vitest-environment happy-dom
import {beforeEach, expect, it} from "vitest";
import {notePageId, type NoteLink} from "@research-assistant/core/notes";
import {createMemoryNotesDatabase} from "../../../runtime/testing/memoryNotesDatabase";
import {selectAppStorage} from "../../../runtime/appStorage";
import {configureBrowserNotesDatabase} from "./browserNotesDatabase";
import {readState, writeState} from "./browserNotesStorage";
import {browserNotesGateway as notes} from "./browserNotesGateway";

beforeEach(() => {localStorage.clear(); selectAppStorage(localStorage);});
it("matches existing outgoing label snapshots including missing targets, without loading bodies or history", async () => {
  const memory = createMemoryNotesDatabase(); configureBrowserNotesDatabase(memory.open);
  const first = await notes.createPage({title:"first"}), second = await notes.createPage({title:"second"});
  const state = await readState();
  const link = (id: string, kind: NoteLink["target"]["kind"], label: string, pageId = first.page.id): NoteLink => ({
    id: id as NoteLink["id"], sourcePageId: pageId, target:{kind, id:"missing-target", labelSnapshot:label}, relation:"reference", isBroken:true, createdAt:"now",
  });
  await writeState({...state, links:[link("a","paper","Paper"),link("b","manuscript","Project"),link("c","note_page","Ignored"),link("d","paper",""),link("e","paper","Other",second.page.id)]});
  const expected = (await notes.listOutgoing(first.page.id)).filter(link => link.target.kind === "paper" || link.target.kind === "manuscript").map(link=>link.target.labelSnapshot).filter(Boolean).join(" · ");
  memory.metrics.documentReads=0; memory.metrics.revisionReads=0;
  expect(await notes.listSourceLabels([first.page.id,first.page.id,notePageId("missing")])).toEqual([{pageId:first.page.id,label:expected}]);
  expect(memory.metrics.documentReads).toBe(0); expect(memory.metrics.revisionReads).toBe(0);
});
it("handles empty input and rejects over-budget input before opening the database", async () => {
  configureBrowserNotesDatabase(async () => {throw Error("unexpected database access");});
  expect(await notes.listSourceLabels([])).toEqual([]);
  await expect(notes.listSourceLabels(Array.from({length:501},()=>notePageId("id")))).rejects.toMatchObject({code:"invalid_input",message:expect.stringContaining("note_source_labels_limit")});
});
