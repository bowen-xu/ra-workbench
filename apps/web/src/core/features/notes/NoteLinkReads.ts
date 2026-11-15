import type { NoteLink, NotePageId } from "@research-assistant/core/notes";
import type { NotesGateway } from "@research-assistant/data/notes";
import type { NoteResourceChange } from "./NoteResourceChanges";

/** Client lifetime, 128 query LRU. Concurrent readers share one gateway call. */
export class NoteLinkReads {
  private readonly reads = new Map<string, Promise<readonly NoteLink[]>>();
  constructor(private readonly gateway: Pick<NotesGateway, "listOutgoing" | "listBacklinks">) {}
  private read(key: string, operation: () => Promise<readonly NoteLink[]>): Promise<readonly NoteLink[]> {
    const found = this.reads.get(key);
    if (found) { this.reads.delete(key); this.reads.set(key, found); return found; }
    const pending = operation().catch(reason => {
      if (this.reads.get(key) === pending) this.reads.delete(key);
      throw reason;
    });
    this.reads.set(key, pending);
    if (this.reads.size > 128) this.reads.delete(this.reads.keys().next().value!);
    return pending;
  }
  listOutgoing = (id: NotePageId): Promise<readonly NoteLink[]> => this.read(`out:${id}`, () => this.gateway.listOutgoing(id));
  listBacklinks = (kind: NoteLink["target"]["kind"], id: string): Promise<readonly NoteLink[]> =>
    this.read(`in:${JSON.stringify([kind, id])}`, () => this.gateway.listBacklinks(kind, id));

  invalidate(change: NoteResourceChange): void {
    if (change.kind === "tags") return;
    if (change.kind === "metadata") { for (const pageId of change.receipt.bodyChangedIds) this.invalidate({kind: "document", pageId, revision: 0}); return; }
    if (change.kind === "all") { this.reads.clear(); return; }
    this.reads.delete(`out:${change.pageId}`);
    // A changed body can add/remove links to any target. Incoming queries are
    // conservatively invalidated; unrelated outgoing queries remain reusable.
    for (const key of this.reads.keys()) if (key.startsWith("in:")) this.reads.delete(key);
  }
}
