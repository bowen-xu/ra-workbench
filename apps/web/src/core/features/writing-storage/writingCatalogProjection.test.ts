import {expect,it} from "vitest";
import fixtures from "../../../../../../tests/contracts/writing-note-catalog-text.json";
import {projectWritingCatalogContent, writingCatalogId} from "./writingCatalogProjection";
import {changeWritingCatalogCounts, emptyWritingCatalogState, type WritingCatalogWork} from "./writingCatalogState";

it.each(fixtures)("uses the shared native writing projection fixture: $name",fixture => {
  const result = projectWritingCatalogContent(JSON.stringify(fixture.content));
  if (!fixture.valid) expect(result).toHaveProperty("error");
  else expect(result).toMatchObject({text: fixture.expectedText});
});
it("retains safe JSON locations and rejects malformed identity/version without disclosing private values", () => {
  const result = projectWritingCatalogContent('{"blocks": SYNTHETIC_PRIVATE_VALUE');
  expect(result).toHaveProperty("error"); expect(JSON.stringify(result)).not.toContain("SYNTHETIC_PRIVATE_VALUE");
  expect(projectWritingCatalogContent(JSON.stringify({schemaVersion:2,noteId:"id",blocks:[]}))).toMatchObject({error:expect.stringContaining("writing_note_version_invalid")});
  expect(projectWritingCatalogContent(JSON.stringify({schemaVersion:1,noteId:"",blocks:[]}))).toMatchObject({error:expect.stringContaining("writing_note_identity_invalid")});
  const id = writingCatalogId('p/:"','n/😀'); expect(id).toBe(`writing:${JSON.stringify(['p/:"','n/😀'])}`);
});
it("moves counts between lifecycle and preparation states without changing authoring facts", () => {
  const original = emptyWritingCatalogState(), pending: WritingCatalogWork = {manuscriptId:"p",path:"a.ranote",sha256:"a".repeat(64),status:"active",state:"pending"};
  const registered = changeWritingCatalogCounts(original,undefined,pending), ready = {...pending,state:"ready" as const,id:writingCatalogId("p","n")};
  const completed = changeWritingCatalogCounts(registered,pending,ready), archived = {...ready,status:"archived" as const};
  const moved = changeWritingCatalogCounts(completed,ready,archived), deleted = changeWritingCatalogCounts(moved,archived,undefined);
  expect(deleted).toEqual(original); expect(original.generation).toBe(0);
  expect(() => changeWritingCatalogCounts(original,ready,undefined)).toThrow("writing_note_catalog_count_invalid");
});
