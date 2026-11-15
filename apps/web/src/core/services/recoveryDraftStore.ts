import { getAppStorage } from "../runtime/appStorage";

export const RECOVERY_DRAFT_SCHEMA_VERSION = 1 as const;

const RECOVERY_DRAFT_PREFIX = "research-assistant.recovery-draft.v1";
const MAX_RECOVERY_DRAFT_BYTES = 8 * 1024 * 1024;

export interface RecoveryDraftIdentity {
  readonly scope: string;
  readonly resourceId: string;
  readonly slot: string;
}

export interface RecoveryDraftEnvelope<T> extends RecoveryDraftIdentity {
  readonly schemaVersion: typeof RECOVERY_DRAFT_SCHEMA_VERSION;
  readonly baseRevision?: number;
  readonly baseHash?: string;
  readonly payload: T;
  readonly updatedAt: string;
}

export class RecoveryDraftStoreError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "RecoveryDraftStoreError";
  }
}

function validateIdentity(identity: RecoveryDraftIdentity): void {
  if (identity.scope.trim().length === 0 || identity.scope.length > 80
    || identity.resourceId.trim().length === 0 || identity.resourceId.length > 500
    || identity.slot.trim().length === 0 || identity.slot.length > 80) {
    throw new RecoveryDraftStoreError("恢复草稿身份无效");
  }
}

function storageOrDefault(storage?: Storage): Storage {
  const resolved = storage ?? (globalThis.window ? getAppStorage() : undefined);
  if (resolved === undefined) throw new RecoveryDraftStoreError("当前环境不支持本地恢复草稿");
  return resolved;
}

export function recoveryDraftKey(identity: RecoveryDraftIdentity): string {
  validateIdentity(identity);
  return [RECOVERY_DRAFT_PREFIX, identity.scope, identity.resourceId, identity.slot]
    .map((part) => encodeURIComponent(part))
    .join(".");
}

export function saveRecoveryDraft<T>(
  draft: RecoveryDraftEnvelope<T>,
  storage?: Storage,
): void {
  validateIdentity(draft);
  if (draft.schemaVersion !== RECOVERY_DRAFT_SCHEMA_VERSION
    || !Number.isFinite(draft.baseRevision ?? 0)
    || Number.isNaN(Date.parse(draft.updatedAt))) {
    throw new RecoveryDraftStoreError("恢复草稿元数据无效");
  }
  let serialized: string;
  try {
    serialized = JSON.stringify(draft);
  } catch (cause) {
    throw new RecoveryDraftStoreError("恢复草稿无法序列化", { cause });
  }
  if (new TextEncoder().encode(serialized).byteLength > MAX_RECOVERY_DRAFT_BYTES) {
    throw new RecoveryDraftStoreError("恢复草稿超过 8 MiB，无法安全暂存");
  }
  try {
    storageOrDefault(storage).setItem(recoveryDraftKey(draft), serialized);
  } catch (cause) {
    throw new RecoveryDraftStoreError("恢复草稿写入本地存储失败", { cause });
  }
}

export function loadRecoveryDraft(
  identity: RecoveryDraftIdentity,
  storage?: Storage,
): RecoveryDraftEnvelope<unknown> | undefined {
  let raw: string | null;
  try {
    raw = storageOrDefault(storage).getItem(recoveryDraftKey(identity));
  } catch (cause) {
    throw new RecoveryDraftStoreError("恢复草稿读取失败", { cause });
  }
  if (raw === null) return undefined;
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch (cause) {
    throw new RecoveryDraftStoreError("恢复草稿内容损坏，原记录已保留", { cause });
  }
  if (typeof value !== "object" || value === null) {
    throw new RecoveryDraftStoreError("恢复草稿格式无效，原记录已保留");
  }
  const candidate = value as Partial<RecoveryDraftEnvelope<unknown>>;
  if (candidate.schemaVersion !== RECOVERY_DRAFT_SCHEMA_VERSION
    || candidate.scope !== identity.scope
    || candidate.resourceId !== identity.resourceId
    || candidate.slot !== identity.slot
    || typeof candidate.updatedAt !== "string"
    || Number.isNaN(Date.parse(candidate.updatedAt))
    || !("payload" in candidate)) {
    throw new RecoveryDraftStoreError("恢复草稿元数据不匹配，原记录已保留");
  }
  return candidate as RecoveryDraftEnvelope<unknown>;
}

export function clearRecoveryDraft(identity: RecoveryDraftIdentity, storage?: Storage): void {
  try {
    storageOrDefault(storage).removeItem(recoveryDraftKey(identity));
  } catch (cause) {
    throw new RecoveryDraftStoreError("恢复草稿清理失败", { cause });
  }
}

export function createRecoveryDraft<T>(
  identity: RecoveryDraftIdentity,
  payload: T,
  base?: { readonly revision?: number; readonly hash?: string },
): RecoveryDraftEnvelope<T> {
  return {
    schemaVersion: RECOVERY_DRAFT_SCHEMA_VERSION,
    ...identity,
    ...(base?.revision === undefined ? {} : { baseRevision: base.revision }),
    ...(base?.hash === undefined ? {} : { baseHash: base.hash }),
    payload,
    updatedAt: new Date().toISOString(),
  };
}
