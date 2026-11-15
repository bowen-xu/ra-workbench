import { expect, it } from "vitest";
import { overviewCandidate } from "../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";
import { mockPaperPortraitGateway } from "./mockPaperPortraitGateway";

it("uses the same paper scope in preview without dropping global history", async () => {
  const own = overviewCandidate("own", "paper"), other = overviewCandidate("other", "other-paper");
  const mixed = { ...own, id: "mixed", evidenceBundle: { ...own.evidenceBundle,
    items: [...own.evidenceBundle.items, ...other.evidenceBundle.items] } };
  const values = new Map([ ["vision-portrait-fixture-version", "1"], ["portrait-candidates", JSON.stringify([own, other, mixed])] ]);
  const storage: Storage = { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); }, clear: () => values.clear() };
  const gateway = mockPaperPortraitGateway(storage);
  await expect(gateway.listCandidates!(undefined, { paperId: "paper" })).resolves.toEqual([own]);
  await expect(gateway.listCandidates!(undefined, { paperId: "other-paper" })).resolves.toEqual([other]);
  await expect(gateway.listCandidates!(undefined, { paperId: "missing" })).resolves.toEqual([]);
  await expect(gateway.listCandidates!()).resolves.toEqual([own, other, mixed]);
});
