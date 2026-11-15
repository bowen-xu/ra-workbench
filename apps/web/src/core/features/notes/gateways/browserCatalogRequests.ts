import {NotesGatewayError} from "@research-assistant/data/notes";

export function catalogRequest<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => { request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error); });
}
export function catalogTransactionDone(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onerror = transaction.onabort = () => reject(transaction.error ?? new NotesGatewayError("storage_error", "note_catalog_transaction_aborted：笔记查询事务已中断。"));
  });
}
