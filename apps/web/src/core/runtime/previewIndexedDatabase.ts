/** Preview buffering consumes a storage capability. Application composition owns
 * IndexedDB schema assembly and domain-specific record registration. */
export interface PreviewDatabaseSnapshot { readonly revision: number; readonly entries: ReadonlyMap<string, string> }
export interface PreviewManagedPersistence {
  readonly pending: boolean;
  subscribe(changed: () => void): () => void;
  flush(): Promise<void>;
  stop(): void;
}
export interface PreviewDatabase {
  readonly managed?: PreviewManagedPersistence;
  readonly bindStorage?: (storage: Storage) => void;
  load(): Promise<PreviewDatabaseSnapshot>;
  readRevision(): Promise<number>;
  commit(revision: number, changes: ReadonlyMap<string, string | null>, clear: boolean): Promise<number>;
  close(): void;
}
export type PreviewDatabaseFactory = (name: string) => Promise<PreviewDatabase>;
let installed: PreviewDatabaseFactory | undefined, override: PreviewDatabaseFactory | undefined;
/** Composition installs the provider; a test override keeps precedence. */
export function installPreviewDatabaseFactory(open: PreviewDatabaseFactory): void { installed = open; }
export function configurePreviewDatabase(open: PreviewDatabaseFactory): void { override = open; }
export function openPreviewDatabase(name: string): Promise<PreviewDatabase> {
  const open = override ?? installed;
  if (!open) return Promise.reject(new PreviewStorageError("preview_storage_unconfigured", "预览存储提供者尚未装配，未打开或修改任何数据库。"));
  return open(name);
}

export class PreviewStorageError extends Error {
  constructor(readonly code: string, message: string, cause?: unknown) {
    super(`${code}：${message}`, cause === undefined ? undefined : {cause});
    this.name = "PreviewStorageError";
  }
}
export function previewStorageError(reason: unknown): PreviewStorageError {
  if (reason instanceof PreviewStorageError) return reason;
  const detail = reason instanceof Error ? `${reason.name}: ${reason.message}` : String(reason);
  return new PreviewStorageError("preview_storage_failed", `IndexedDB 写入未完成，已有数据和待写内容均保留。请重试；刷新前请先完成保存。${detail}`, reason);
}
