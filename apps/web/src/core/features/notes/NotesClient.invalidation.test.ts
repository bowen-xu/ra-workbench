import {expect, it, vi} from "vitest";
import {notePageId} from "@research-assistant/core/notes";
import type {NotesGateway} from "@research-assistant/data/notes";
import {NotesClient} from "./NotesClient";

const id = notePageId("a"), other = notePageId("b");
it("deduplicates the same saved receipt, scopes tag invalidation, and bounds identity history", () => {
  const client = new NotesClient({} as NotesGateway), listener = vi.fn(); client.subscribe(listener);
  client.notifyChanged({kind: "document", pageId: id, revision: 1});
  client.notifyChanged({kind: "document", pageId: id, revision: 1});
  expect(listener).toHaveBeenCalledOnce(); expect(client.getTagsVersion()).toBe(0);
  expect(client.getChangesSince(0)).toEqual([{kind: "document", pageId: id, revision: 1}]);
  client.notifyChanged(); client.notifyChanged({kind: "document", pageId: id, revision: 1});
  expect(listener).toHaveBeenCalledTimes(3); expect(client.getTagsVersion()).toBe(1);
  for(let revision = 2; revision < 70; revision++) client.notifyChanged({kind: "document", pageId: id, revision});
  expect(client.getChangesSince(0)).toEqual([{kind: "all"}]);
  expect(client.getChangesSince(client.getVersion())).toEqual([]);
});
it("shares relation queries, invalidates changed outgoing and all incoming, and retries rejected reads", async () => {
  const outgoing = vi.fn(async () => []), incoming = vi.fn(async () => []);
  const client = new NotesClient({listOutgoing: outgoing, listBacklinks: incoming} as unknown as NotesGateway);
  await Promise.all([client.links.listOutgoing(id), client.links.listOutgoing(id), client.links.listOutgoing(other), client.links.listBacklinks("note_page", other)]);
  expect(outgoing).toHaveBeenCalledTimes(2); expect(incoming).toHaveBeenCalledOnce();
  client.notifyChanged({kind: "document", pageId: id, revision: 1});
  await Promise.all([client.links.listOutgoing(id), client.links.listOutgoing(other), client.links.listBacklinks("note_page", other)]);
  expect(outgoing).toHaveBeenCalledTimes(3); expect(incoming).toHaveBeenCalledTimes(2);
  client.notifyChanged(); outgoing.mockRejectedValueOnce(new Error("offline"));
  await expect(client.links.listOutgoing(id)).rejects.toThrow("offline");
  await expect(client.links.listOutgoing(id)).resolves.toEqual([]);
});
