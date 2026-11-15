import {agentDraftFromInput, agentDraftInput, decodeAgentUserInput} from "@research-assistant/core/agent";
import {decodeQueueDraftPlacement, type ConversationInputDraft} from "./queueComposerDraft";
import {flushStorage, storagePersistence} from "../../runtime/storagePersistence";

export interface ConversationDraftSnapshotEntry {
  readonly draft: ConversationInputDraft;
  readonly revision: string | null;
  readonly originKey: string | null;
  /** Local placement ownership, independent of the service's numeric queue revision. */
  readonly sourceQueueRevision: string | null;
}
export type ConversationDraftSnapshot = ReadonlyMap<string, ConversationDraftSnapshotEntry>;
export interface ConversationDraftPersistence {
  load(): ConversationDraftSnapshot;
  save(snapshot: ConversationDraftSnapshot): void;
  flush?(): Promise<void>;
  retry?(): Promise<void>;
}
const uuid = (value: unknown): value is string => typeof value === "string"
  && /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/u.test(value) && !/^0{8}-(?:0{4}-){3}0{12}$/u.test(value);
function scope(key: unknown): readonly [string, string, string | null] {
  if (typeof key !== "string") throw new Error("草稿身份无效");
  const value: unknown = JSON.parse(key);
  if (!Array.isArray(value) || value.length !== 3 || !["manuscript", "paper", "note", "readingNote"].includes(value[0])
    || typeof value[1] !== "string" || (value[2] !== null && typeof value[2] !== "string") || JSON.stringify(value) !== key) throw new Error("草稿身份无效");
  return value as [string, string, string | null];
}
function validate(snapshot: ConversationDraftSnapshot): void {
  if (snapshot.size > 256) throw new Error("草稿数量超过预算");
  const revisions = new Set<string>(), placements = new Set<string>();
  for (const [key, entry] of snapshot) {
    const current = scope(key);
    if (entry.revision === null) {
      if (entry.originKey !== null || entry.sourceQueueRevision !== null) throw new Error("草稿修订无效");
    } else {
      if (!uuid(entry.revision) || revisions.has(entry.revision)) throw new Error("草稿修订无效"); revisions.add(entry.revision);
      const origin = scope(entry.originKey);
      if (origin[0] !== current[0] || origin[1] !== current[1] || (origin[2] !== null && origin[2] !== current[2])) throw new Error("草稿归属无效");
    }
    if (entry.sourceQueueRevision !== null) {
      if (!entry.draft.queuePlacement || !uuid(entry.sourceQueueRevision) || placements.has(entry.sourceQueueRevision)) throw new Error("草稿队列归属无效");
      placements.add(entry.sourceQueueRevision);
    }
  }
}

/** One read/write contains both content and provenance. v1 stays unversioned
 * until a real edit or an explicitly fresh capture creates a new revision. */
export function localConversationDraftPersistence(storage: Pick<Storage, "getItem" | "setItem">): ConversationDraftPersistence {
  const key = "research-assistant.agent-drafts.v1";
  return {
    load: () => {
      const text = storage.getItem(key); if (!text) return new Map();
      if (new TextEncoder().encode(text).length > 2_097_152) throw new Error("草稿存储超过预算");
      const value: unknown = JSON.parse(text);
      if (!value || typeof value !== "object" || Array.isArray(value) || !("version" in value) || (value.version !== 1 && value.version !== 2)
        || !("drafts" in value) || !Array.isArray(value.drafts) || value.drafts.length > 256) throw new Error("草稿存储无法读取");
      const snapshot = new Map<string, ConversationDraftSnapshotEntry>();
      for (const raw of value.drafts) {
        if (value.version === 1) {
          if (!Array.isArray(raw) || (raw.length !== 3 && raw.length !== 4) || typeof raw[0] !== "string" || typeof raw[1] !== "string" || snapshot.has(raw[0])) throw new Error("草稿身份无效");
          scope(raw[0]); snapshot.set(raw[0], {draft: {...agentDraftFromInput(raw[1], raw[2]), ...(raw.length === 4 ? {queuePlacement: decodeQueueDraftPlacement(raw[3])} : {})},
            revision: null, originKey: null, sourceQueueRevision: null});
        } else {
          if (!raw || typeof raw !== "object" || Array.isArray(raw)) throw new Error("草稿存储无法读取");
          const row = raw as Record<string, unknown>;
          if (Object.keys(row).sort().join() !== "key,originKey,queuePlacement,revision,sourceQueueRevision,text,userInput" || typeof row.key !== "string"
            || typeof row.text !== "string" || snapshot.has(row.key)) throw new Error("草稿身份无效");
          if (!Array.isArray(row.userInput)) throw new Error("草稿输入无效");
          const draft = {...agentDraftFromInput(row.text, decodeAgentUserInput(row.userInput, row.text)),
            ...(row.queuePlacement !== null ? {queuePlacement: decodeQueueDraftPlacement(row.queuePlacement)} : {})};
          if ((row.revision !== null && !uuid(row.revision)) || (row.originKey !== null && typeof row.originKey !== "string")
            || (row.sourceQueueRevision !== null && !uuid(row.sourceQueueRevision))) throw new Error("草稿修订无效");
          snapshot.set(row.key, {draft, revision: row.revision, originKey: row.originKey, sourceQueueRevision: row.sourceQueueRevision});
        }
      }
      validate(snapshot); return snapshot;
    },
    save: snapshot => {
      validate(snapshot);
      const text = JSON.stringify({version: 2, drafts: [...snapshot].map(([key, entry]) => ({key, text: entry.draft.text,
        userInput: agentDraftInput(entry.draft), queuePlacement: entry.draft.queuePlacement ?? null,
        revision: entry.revision, originKey: entry.originKey, sourceQueueRevision: entry.sourceQueueRevision}))});
      if (new TextEncoder().encode(text).length > 2_097_152) throw new Error("草稿存储超过预算"); storage.setItem(key, text);
    },
    flush: () => flushStorage(storage as Storage),
    retry: async () => {await storagePersistence(storage as Storage)?.retry();},
  };
}
