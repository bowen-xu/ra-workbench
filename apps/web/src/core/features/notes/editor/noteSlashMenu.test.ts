import { describe, expect, it } from "vitest";
import type { NoteEditorInstance } from "./noteEditorSchema";
import type { DefaultSuggestionItem } from "@blocknote/core/extensions";

import { buildNoteEditingItems, filterLosslessDefaultSlashItems, SAFE_DEFAULT_SLASH_KEYS } from "../../../components/block-editor/noteEditingCommands";

describe("note slash menu", () => {
  it("only exposes default blocks that NoteDocumentV1 persists losslessly", () => {
    const items = ["paragraph", "heading", "toggle_list", "image", "audio", "divider"].map((key) => ({
      key,
      title: key,
      onItemClick: () => undefined,
    })) as DefaultSuggestionItem[];
    expect(filterLosslessDefaultSlashItems(items).map((item) => item.title)).toEqual(["paragraph", "heading", "toggle_list", "divider"]);
    expect(SAFE_DEFAULT_SLASH_KEYS).toContain("toggle_list");
    expect(SAFE_DEFAULT_SLASH_KEYS).not.toContain("image");
  });
  it("shows typed input sequences instead of platform keyboard chords", () => {
    const items = ["heading", "heading_2", "heading_3", "heading_4", "heading_5", "heading_6", "quote", "check_list", "paragraph"].map(key => ({ key, title: key, badge: "Cmd-Alt-1", onItemClick: () => undefined })) as DefaultSuggestionItem[];
    expect(filterLosslessDefaultSlashItems(items).map(item => item.badge)).toEqual(["#", "##", "###", "####", '"', "[]", undefined]);
  });
});

it("keeps reference targets in the mention menu and inserts each as inline content", async () => {
  const { BlockNoteEditor } = await import("@blocknote/core");
  const { noteEditorSchema } = await import("./noteEditorSchema");
  const { buildNoteDomainSlashItems, buildResearchMentionItems } = await import("./noteSlashMenu");
  const editor = BlockNoteEditor.create({ schema: noteEditorSchema }) as unknown as NoteEditorInstance;
  const commands = [...buildNoteEditingItems(editor), ...buildNoteDomainSlashItems({ editor, onChooseAsset: () => undefined, onCreatePage: () => undefined })];
  expect(commands.some(item => item.group?.includes("引用"))).toBe(false);
  expect(commands.map(item => item.title)).toContain("Quote"); // Block quote is an editing command.
  expect(commands.map(item => item.title)).toContain("新页面");
  const targets = [
    { kind: "note_page" as const, id: "page-1", label: "Plan" },
    { kind: "reading_note" as const, id: "reading-1", label: "Evidence", context: "Quote" },
    { kind: "paper" as const, id: "paper-1", label: "Paper" },
  ];
  const mentions = buildResearchMentionItems(editor, targets);
  expect(mentions.map(item => item.group)).toEqual(["页面引用", "阅读笔记引用", "研究对象"]);
  for (let index = 0; index < targets.length; index++) {
    const engine = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: [{ type: "paragraph", content: "Before " }] }) as unknown as NoteEditorInstance;
    engine.setTextCursorPosition(engine.document[0]!, "end");
    buildResearchMentionItems(engine, targets)[index]!.onItemClick();
    expect(engine.document[0]!.type).toBe("paragraph");
    expect(engine.document[0]!.content).toEqual([
      { type: "text", text: "Before ", styles: {} },
      { type: "researchMention", props: { targetKind: targets[index]!.kind, targetId: targets[index]!.id, labelSnapshot: targets[index]!.label },
        content: [{ type: "text", text: targets[index]!.label, styles: { bold: true } }] },
      { type: "text", text: " ", styles: {} },
    ]);
  }
});

it("inserts native toggles and toggle headings through the slash commands", async () => {
  const {BlockNoteEditor} = await import("@blocknote/core");
  const {baseBlockEditorSchema} = await import("../../../components/block-editor/baseSchema");
  const {zh} = await import("@blocknote/core/locales");
  for (const title of ["可折叠列表", "可折叠二级标题"]) {
    const editor = BlockNoteEditor.create({schema: baseBlockEditorSchema, dictionary: zh});
    const item = buildNoteEditingItems(editor).find(item => item.title === title);
    expect(item).toBeDefined(); item!.onItemClick();
    expect(editor.document[0]).toMatchObject(title === "可折叠列表"
      ? {type: "toggleListItem"} : {type: "heading", props: {level: 2, isToggleable: true}});
  }
});
