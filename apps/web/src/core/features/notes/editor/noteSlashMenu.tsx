import { closeHistory } from "@tiptap/pm/history";
import {
  FilePlus2,
  FileUp,
  ImagePlus,
  Quote,
  FileSymlink,
} from "lucide-react";
import {
  filterSuggestionItems,
} from "@blocknote/core/extensions";
import type { DefaultReactSuggestionItem } from "@blocknote/react";

import type { ResearchObjectTarget } from "../links/useResearchObjectTargets";
import type { NoteEditorInstance } from "./noteEditorSchema";

interface NoteSlashMenuOptions {
  readonly editor: NoteEditorInstance;
  readonly onChooseAsset: () => void;
  readonly onCreatePage?: () => void;
  readonly onChoosePageLink?: () => void;
}

export function buildNoteDomainSlashItems(options: NoteSlashMenuOptions): DefaultReactSuggestionItem[] {
  const customItems: DefaultReactSuggestionItem[] = [
    {
      title: "图片或附件",
      subtext: "导入并由笔记库管理本地文件",
      aliases: ["image", "file", "attachment", "上传", "附件"],
      group: "研究内容",
      icon: <ImagePlus size={18} />,
      onItemClick: options.onChooseAsset,
    },
  ];
  if (options.onChoosePageLink) customItems.push({ title: "链接到笔记", subtext: "将已有笔记作为一个链接块插入", aliases: ["link", "link to page", "page link", "链接", "笔记链接"], group: "研究内容", icon: <FileSymlink size={18} />, onItemClick: options.onChoosePageLink });

  if (options.onCreatePage) customItems.push({
    title: "新页面", subtext: "创建独立笔记，并在当前位置插入链接", aliases: ["new page", "新页面", "新笔记"],
    group: "研究内容", icon: <FilePlus2 size={18} />, onItemClick: options.onCreatePage,
  });

  return customItems;
}

type NoteReferenceTarget = Omit<ResearchObjectTarget, "kind"> & {
  readonly kind: ResearchObjectTarget["kind"] | "note_page" | "reading_note";
};

export function buildResearchMentionItems(
  editor: NoteEditorInstance,
  targets: readonly NoteReferenceTarget[],
): DefaultReactSuggestionItem[] {
  return targets.map((target) => ({
    title: target.kind === "note_page" ? `页面 · ${target.label}`
      : target.kind === "reading_note" ? `阅读笔记 · ${target.label}` : target.label,
    ...(target.context === undefined ? {} : { subtext: target.context }),
    aliases: [target.label, target.kind, ...(target.kind === "note_page" ? ["页面", "page"]
      : target.kind === "reading_note" ? ["阅读笔记", "reading note"] : ["研究对象"])],
    group: target.kind === "note_page" ? "页面引用" : target.kind === "reading_note" ? "阅读笔记引用" : "研究对象",
    ...(target.kind === "note_page" ? { icon: <FileUp size={16} /> }
      : target.kind === "reading_note" ? { icon: <Quote size={16} /> } : {}),
    onItemClick: () => {
      editor.transact(transaction => { closeHistory(transaction); });
      editor.insertInlineContent([{
        type: "researchMention",
        props: { targetKind: target.kind, targetId: target.id, labelSnapshot: target.label },
        content: [{ type: "text", text: target.label, styles: { bold: true } }],
      }, " "], { updateSelection: true });
      editor.transact(transaction => { closeHistory(transaction); });
    },
  }));
}

export function filterNoteSuggestionItems(items: DefaultReactSuggestionItem[], query: string): DefaultReactSuggestionItem[] {
  return filterSuggestionItems(items, query);
}
