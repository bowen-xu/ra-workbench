import {noteSearchGramDelta, updateNoteSearchPostingBatch} from "@research-assistant/data/notes";
import {readBrowserCatalogPosting,writeBrowserCatalogPosting} from "./browserCatalogPostingRows";

/** Components in one canonical/projection transaction share posting rows.
 * Call flush before returning from that transaction's projection owner. No
 * cache survives the transaction or supplies an authoring/save receipt. */
export function catalogPostingUpdater(transaction: IDBTransaction) {
  const rows = new Map<string,{value: Uint32Array;legacy: boolean;updates: Map<number,boolean>}>();
  const update = async (component: number, before: ReadonlySet<string>, after: ReadonlySet<string>): Promise<void> => {
    const delta = noteSearchGramDelta(before, after);
    for (const [include, grams] of [[false,delta.removed],[true,delta.added]] as const) for (const gram of grams) {
      let row = rows.get(gram);
      if (!row) {
        row = {...await readBrowserCatalogPosting(transaction,gram),updates: new Map()};
        rows.set(gram,row);
      }
      row.updates.set(component,include);
    }
  };
  return Object.assign(update,{flush(): void {
    for (const [gram,row] of rows) {
      const changed = updateNoteSearchPostingBatch(row.value,row.updates);
      if (changed !== row.value) writeBrowserCatalogPosting(transaction,gram,changed,row.legacy);
    }
    rows.clear();
  }});
}
