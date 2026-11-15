
/** Invalidation identities only: live editor bodies are never saved index data. */
export type { NoteResourceChange } from "../../contracts/noteResources";
import type { NoteResourceChange } from "../../contracts/noteResources";

export class NoteResourceChanges {
  private readonly history: {version: number; change: NoteResourceChange}[] = [];
  private version = 0;
  private tagsVersion = 0;
  getVersion = (): number => this.version;
  getTagsVersion = (): number => this.tagsVersion;

  record(change: NoteResourceChange): boolean {
    if (change.kind === "document") {
      for (let index = this.history.length - 1; index >= 0; index--) {
        const entry = this.history[index]!;
        if (entry.change.kind === "all") break;
        if (entry.change.kind === "document" && entry.change.pageId === change.pageId) {
          if (entry.change.revision === change.revision) return false;
          break;
        }
      }
    }
    this.version++;
    if (change.kind === "tags" || change.kind === "all") this.tagsVersion++;
    this.history.push({version: this.version, change});
    if (this.history.length > 64) this.history.shift();
    return true;
  }

  since(version: number): readonly NoteResourceChange[] {
    if (version === this.version) return [];
    if (version > this.version || version < (this.history[0]?.version ?? this.version) - 1) return [{kind: "all"}];
    return this.history.filter(entry => entry.version > version).map(entry => entry.change);
  }
}
