import { type BrowserNotesDatabase, type StoredRows, type NoteRead, type NoteWrite, type NoteDocumentSelector,type NoteIndexedRead } from "../../features/notes/gateways/browserNotesDatabase";
import { NotesGatewayError } from "@research-assistant/data/notes";

/** Unit tests inject persistence; production IndexedDB is exercised in a browser. */
export function createMemoryNotesDatabase() {
  const databases = new Map<string, {version: number; rows: StoredRows}>();
  const metrics = {reads: 0, commits: 0, documentReads: 0, documentWrites: 0, revisionReads: 0};
  const open = async (name: string): Promise<BrowserNotesDatabase> => {
    const data = databases.get(name) ?? {version: 0, rows: {}};
    databases.set(name, data);
    return {
      async read(query: NoteRead, selectDocuments?: NoteDocumentSelector) {
        if (selectDocuments && query.documents !== undefined) throw new NotesGatewayError("invalid_input", "conflicting document queries");
        metrics.reads++;
        const rows: StoredRows = {};
        for (const [store, keys] of Object.entries(query) as [keyof StoredRows, NonNullable<NoteRead[keyof NoteRead]>][]) {
          const found = data.rows[store] ?? {};
          const indexed=keys as NoteIndexedRead;
          const selected = keys === "all" ? found : Array.isArray(keys)?Object.fromEntries(keys.filter(key => key in found).map(key => [key, found[key]])):
            Object.fromEntries(Object.entries(found).filter(([key,value])=>indexed.primaryKeys?.includes(key)||indexed.keys.includes((value as Record<string,string>)[indexed.index]!)));
          rows[store] = structuredClone(selected);
          if (store === "documents") metrics.documentReads += Object.keys(selected).length;
          if (store === "revisions") metrics.revisionReads += Object.keys(selected).length;
        }
        if (selectDocuments) {
          const found = data.rows.documents ?? {};
          rows.documents = Object.fromEntries([...new Set(selectDocuments(rows, data.version))].filter(key => key in found).map(key => [key, structuredClone(found[key])]));
          metrics.documentReads += Object.keys(rows.documents).length;
        }
        return {version: data.version, rows};
      },
      async commit(version: number, changes: readonly NoteWrite[]) {
        if (version !== data.version) throw new NotesGatewayError("revision_conflict", "database changed", data.version);
        const rows = {...data.rows};
        const copied = new Set<string>();
        for (const change of changes) {
          if (!copied.has(change.store)) { rows[change.store] = {...rows[change.store]}; copied.add(change.store); }
          const store = rows[change.store]!;
          if (change.value === undefined) delete store[change.key]; else store[change.key] = structuredClone(change.value);
          if (change.store === "documents") metrics.documentWrites++;
        }
        data.rows = rows; data.version++; metrics.commits++;
      },
    };
  };
  return {open, metrics};
}
