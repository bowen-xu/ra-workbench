import { describe, expect, it } from "vitest";

import {
  RecoveryDraftStoreError,
  clearRecoveryDraft,
  createRecoveryDraft,
  loadRecoveryDraft,
  recoveryDraftKey,
  saveRecoveryDraft,
} from "./recoveryDraftStore";

class MemoryStorage implements Storage {
  readonly #values = new Map<string, string>();
  get length(): number { return this.#values.size; }
  clear(): void { this.#values.clear(); }
  getItem(key: string): string | null { return this.#values.get(key) ?? null; }
  key(index: number): string | null { return [...this.#values.keys()][index] ?? null; }
  removeItem(key: string): void { this.#values.delete(key); }
  setItem(key: string, value: string): void { this.#values.set(key, value); }
}

const identity = { scope: "note-page", resourceId: "page/one", slot: "document" } as const;

describe("recovery draft store", () => {
  it("round-trips a versioned draft under a stable encoded identity", () => {
    const storage = new MemoryStorage();
    const draft = createRecoveryDraft(identity, { body: "未保存内容" }, { revision: 4 });
    saveRecoveryDraft(draft, storage);

    expect(recoveryDraftKey(identity)).not.toContain("page/one");
    expect(loadRecoveryDraft(identity, storage)).toEqual(draft);
    clearRecoveryDraft(identity, storage);
    expect(loadRecoveryDraft(identity, storage)).toBeUndefined();
  });

  it("preserves malformed source data and reports a recoverable error", () => {
    const storage = new MemoryStorage();
    const key = recoveryDraftKey(identity);
    storage.setItem(key, "not-json");

    expect(() => loadRecoveryDraft(identity, storage)).toThrow(RecoveryDraftStoreError);
    expect(storage.getItem(key)).toBe("not-json");
  });

  it("reports storage write failure instead of treating the draft as durable", () => {
    const storage = new MemoryStorage();
    storage.setItem = () => { throw new DOMException("quota", "QuotaExceededError"); };
    expect(() => saveRecoveryDraft(createRecoveryDraft(identity, "draft"), storage))
      .toThrow("恢复草稿写入本地存储失败");
  });
});
