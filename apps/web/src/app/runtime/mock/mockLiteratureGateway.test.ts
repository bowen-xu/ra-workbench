// @vitest-environment happy-dom
import { beforeEach, expect, it, vi } from "vitest";
import { emptyLiteratureDraft, type LiteratureDraft, type LiteratureItem } from "@research-assistant/core/literature";
import type { LiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";
import { createMockLiteratureGateway } from "./mockLiteratureGateway";

beforeEach(() => localStorage.clear());
function setup() {
  const items = new Map<string, LiteratureItem>();
  const save = vi.fn(async (draft: LiteratureDraft): Promise<LiteratureItem> => {
    const item: LiteratureItem = { ...draft, id: draft.id ?? crypto.randomUUID(), attachments: [], projectIds: [], metadataSources: [],
      readingStatus: "unread", createdAt: "now", updatedAt: "now" };
    items.set(item.id, item);
    return item;
  });
  const base = { save, get: async (id: string) => items.get(id), list: async () => [...items.values()] } as unknown as LiteratureGateway;
  return { gateway: createMockLiteratureGateway(base, localStorage), base, save };
}

it("imports a DOI in an empty preview library only after confirmation, with a real task lifecycle", async () => {
  const { gateway, base, save } = setup();
  const preview = await gateway.previewDoi("10.1234/test");
  expect(save).not.toHaveBeenCalled();
  expect(await gateway.getImportJob(preview.importJobId)).toMatchObject({ state: "queued", completedCount: 0 });
  const item = await gateway.applyMetadataCandidate(preview.id, preview.draft);
  expect((await base.list()).map(item => item.id)).toEqual([item.id]);
  expect(await gateway.getImportJob(preview.importJobId)).toMatchObject({ state: "completed", completedCount: 1,
    results: [expect.objectContaining({ state: "imported", itemId: item.id })] });
  await expect(gateway.applyMetadataCandidate(preview.id, preview.draft)).rejects.toThrow("已过期");
  expect(save).toHaveBeenCalledOnce();
});

it("never borrows an existing item's identity when importing a new DOI", async () => {
  const { gateway, base } = setup();
  const original = await base.save({ ...emptyLiteratureDraft(), title: "Original" });
  const preview = await gateway.previewDoi("10.1234/new");
  expect(preview.draft.id).toBeUndefined();
  const added = await gateway.applyMetadataCandidate(preview.id, preview.draft);
  expect(added.id).not.toBe(original.id);
  expect(await base.get(original.id)).toMatchObject({ title: "Original" });
  expect(await base.list()).toHaveLength(2);
});

it("rejects a pending candidate and closes its job without creating a literature item", async () => {
  const { gateway, save } = setup();
  const preview = await gateway.previewDoi("10.1234/cancel");
  await gateway.rejectMetadataCandidate(preview.id);
  expect(await gateway.getImportJob(preview.importJobId)).toMatchObject({ state: "cancelled", cancellationRequested: true });
  expect(save).not.toHaveBeenCalled();
});

it("updates an explicitly selected item when applying field decisions and finishes the job", async () => {
  const { gateway, base } = setup();
  const item = await base.save({ ...emptyLiteratureDraft(), title: "Before" });
  const preview = await gateway.previewDoi("10.1234/update", item.id);
  await gateway.applyMetadataCandidateFields(preview.id, [{ fieldName: "title", choice: "candidate" }], "trusted");
  expect(await base.list()).toHaveLength(1);
  expect(await base.get(item.id)).toMatchObject({ title: "模拟 DOI 元数据" });
  expect(await gateway.getImportJob(preview.importJobId)).toMatchObject({ state: "completed" });
});
