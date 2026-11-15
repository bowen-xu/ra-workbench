import { isStoredWordExplanation, type StoredWordExplanation, type WordExplanationDraft, type WordExplanationStore } from "@research-assistant/core/dictionary";
import { getAppStorage } from "../runtime/appStorage";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import { resolveAppGateway } from "../runtime/appRuntime";

function decode(value: unknown): StoredWordExplanation {
  if (!isStoredWordExplanation(value)) throw new Error("词义解释存储格式无效。");
  return value;
}
function errorMessage(error: unknown, fallback = "词义解释保存失败，请重试。"): Error {
  return new Error(typeof error === "object" && error !== null && "message" in error && typeof error.message === "string" ? error.message : fallback);
}
const desktop: WordExplanationStore = {
  async remove(record) {
    try { await invoke("delete_word_explanation", { id: record.id, attachmentId: record.attachmentId, contentSha256: record.contentSha256 }); }
    catch (error) { throw errorMessage(error, "删除词义解释失败，请重试。"); }
  },
  async list(attachmentId, contentSha256) {
    const value = await invoke<unknown>("list_word_explanations", { attachmentId, contentSha256 });
    if (!Array.isArray(value)) throw new Error("词义解释列表格式无效。");
    return value.map(decode);
  },
  async save(draft) {
    try { return decode(await invoke("save_word_explanation", { draft })); }
    catch (error) { throw errorMessage(error); }
  },
};
function key(attachmentId: string): string { return `word-explanations-v1:${attachmentId}`; }
/** Local browser persistence port, also injected into the production preview tree. */
export function createBrowserWordExplanationStore(): WordExplanationStore {
  function read(attachmentId: string): StoredWordExplanation[] {
    const raw = getAppStorage().getItem(key(attachmentId));
    if (raw === null) return [];
    const value: unknown = JSON.parse(raw);
    if (!Array.isArray(value)) throw new Error("本地词义解释列表无效。");
    return value.map(decode);
  }
  return {
    async remove(record) { getAppStorage().setItem(key(record.attachmentId), JSON.stringify(read(record.attachmentId).filter(value => value.id !== record.id))); },
    async list(attachmentId, contentSha256) { return read(attachmentId).filter(value => value.contentSha256 === contentSha256); },
    async save(draft: WordExplanationDraft) {
      const values = read(draft.attachmentId);
      const sameSource = (value: StoredWordExplanation): boolean => value.contentSha256 === draft.contentSha256 && JSON.stringify(value.segments) === JSON.stringify(draft.segments);
      const record = decode({ ...draft, id: values.find(sameSource)?.id ?? crypto.randomUUID() });
      getAppStorage().setItem(key(draft.attachmentId), JSON.stringify([...values.filter(value => !sameSource(value)), record]));
      return record;
    },
  };
}
const browser = createBrowserWordExplanationStore();
export function getWordExplanationStore(): WordExplanationStore {
  return resolveAppGateway("getWordExplanationStore", isDesktopRuntime() ? desktop : browser);
}
