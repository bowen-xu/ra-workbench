// @vitest-environment happy-dom
import {beforeAll, expect, it} from "vitest";
import {createNoteDocumentAdapter, createStructuredDocumentAdapter, createTextDocumentAdapter, type HbsDocumentAdapter} from "@research-assistant/core/hbs";
import {configurePreviewDatabase} from "../../../core/runtime/previewIndexedDatabase";
import {createMemoryPreviewDatabase} from "../../runtime/testing/memoryPreviewDatabase";
import {directTextDiff} from "../../../core/components/hbs/codeMirrorHbsPolicy";
import {noteFixture, structuredFixture, textFixture} from "./fixtures";
import {openExample, previewCall} from "./storage";
import {changeNoteExample} from "./noteExternalChanges";
import {resetExample} from "./resetExample";
import {fixtureAssets} from "./fixtureAssets";

beforeAll(() => configurePreviewDatabase(createMemoryPreviewDatabase().open));
const cases = [
  {adapter: createTextDocumentAdapter(directTextDiff), fixture: textFixture, incoming: textFixture.replace("80%", "90%")},
  {adapter: createNoteDocumentAdapter(directTextDiff), fixture: noteFixture, incoming: changeNoteExample(noteFixture, "paragraph", 2)},
  {adapter: createStructuredDocumentAdapter(directTextDiff), fixture: structuredFixture, incoming: {...structuredFixture,
    fields: structuredFixture.fields.map((field, index) => index ? field : {...field, statements: field.statements.map(statement => ({...statement, text: "外部候选"}))})}},
];
it.each(cases)("persists and restores $adapter.kind review through the injected ports and verifies reset", async ({adapter: input, fixture, incoming}) => {
  const adapter = input as HbsDocumentAdapter<unknown>;
  const first = await openExample(adapter, fixture);
  try {
    const initial = await first.actual.read();
    await first.actual.commit({id: crypto.randomUUID(), expectedVersion: initial.version, saved: incoming, baseline: initial.baseline});
    await first.session.syncLatest(); await first.session.save(); await first.session.flushPreservation();
    expect(first.session.getSnapshot().changes.length).toBeGreaterThan(0);
    first.session.dispose(); first.delivery.dispose();
    const restored = await openExample(adapter, fixture);
    try {
      expect(restored.session.getSnapshot().content).toEqual(adapter.decode(incoming));
      expect(restored.session.getSnapshot().changes.length).toBeGreaterThan(0);
      expect((await restored.actual.read()).baseline).toEqual(adapter.decode(fixture));
      const inspected = await restored.inspect();
      expect(inspected.storage.S[0]?.columns).toEqual(["key", "value"]);
      await resetExample({...restored, adapter, fixture, external: [],
        clearPreservation: async () => {await previewCall(adapter.kind, "clearPreservation", restored.id);},
        readPreservation: () => previewCall(adapter.kind, "readPreservation", restored.id),
      });
      expect(await previewCall(adapter.kind, "readPreservation", restored.id)).toBeNull();
    } finally {restored.session.dispose(); restored.delivery.dispose();}
  } finally {first.session.dispose(); first.delivery.dispose();}
});

it("keeps synthetic attachment identity and bytes available across reopen without a server", async () => {
  expect(await previewCall("note", "fixtureAssets")).toEqual(fixtureAssets);
  const bytes = await previewCall("note", "readAsset", fixtureAssets.image.id) as number[];
  expect(bytes.slice(0, 4)).toEqual([137, 80, 78, 71]);
  expect(await previewCall("note", "readAsset", fixtureAssets.file.id)).toEqual([...new TextEncoder().encode("Synthetic HBS attachment\n")]);
});
