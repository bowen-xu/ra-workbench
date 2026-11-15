import type { NoteSaveStatus } from "../editor/noteSaveController";
import type { ReadingNoteBlock } from "@research-assistant/core/notes";

export function readingNoteTypeLabel(type: ReadingNoteBlock["noteType"]): string {
  return type === "highlight" ? "标注" : "便签";
}

export function formatNoteDate(value: string): string {
  return new Intl.DateTimeFormat("zh-CN", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(new Date(value));
}

export function noteSaveLabel(status: NoteSaveStatus): string {
  return { saved: "已保存", dirty: "等待保存", saving: "正在保存…", failed: "保存失败", conflict: "版本冲突" }[status];
}
