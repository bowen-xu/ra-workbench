import {packCompletedText,unpackCompletedText} from "./mockCompletedText";

/** Only synthetic mock state uses this codec; binary attachments remain URLs. */
export function encodeMockState(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) => {
    if (item instanceof Map) return { mockType: "map", entries: [...item] };
    if (item instanceof Set) return { mockType: "set", entries: [...item] };
    if (item instanceof Uint8Array) return { mockType: "bytes", entries: [...item] };
    const completed=packCompletedText(item);if(completed)return completed;
    return item;
  });
}
export function decodeMockState<T>(value: string): T {
  return JSON.parse(value, (_key, item: unknown) => {
    if (!item || typeof item !== "object" || !("mockType" in item) || !("entries" in item)) return item;
    if (!Array.isArray(item.entries)) throw new Error("预览快照格式错误");
    switch (item.mockType) {
      case "map": return new Map(item.entries);
      case "set": return new Set(item.entries);
      case "bytes": return Uint8Array.from(item.entries);
      case "agentEvents": return unpackCompletedText({entries:item.entries,messages:"messages" in item?item.messages:undefined});
      default: throw new Error("未知预览快照类型");
    }
  }) as T;
}
