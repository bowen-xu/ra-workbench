import { createContext, useLayoutEffect, useRef, useState, type ReactElement } from "react";
import { ArrowRightLeft, Check, ChevronRight, Link2 } from "lucide-react";
import { closeHistory } from "@tiptap/pm/history";
import { formatSourceLink } from "@research-assistant/core/source";
import { NoteActionMenu as ActionMenu, NoteActionMenuItem as ActionMenuItem } from "./NoteActionMenu";
import { notify } from "../../services/notifications";
import type { baseBlockEditorSchema } from "./baseSchema";
import "./NoteBlockActions.css";
import { NOTE_BLOCK_TYPES } from "./noteBlockTypes";

export const NoteBlockActionsContext = createContext<{readonly sourcePath?: readonly string[]} | undefined>(undefined);
type Editor = typeof baseBlockEditorSchema.BlockNoteEditor;
const conversions = NOTE_BLOCK_TYPES;
const textTypes = new Set(conversions.map(item => item.type));

/** The hovered block owns the command, independently of the text caret. Hosts
 * supply only a virtual source path; this menu never reads note persistence. */
export function NoteBlockActions({editor, blockId, sourcePath, trigger, setMenuOpen}: {
  readonly editor: Editor;
  readonly blockId: string;
  readonly sourcePath?: readonly string[];
  readonly trigger: ReactElement;
  readonly setMenuOpen: (open: boolean) => void;
}): React.JSX.Element {
  const [open, setOpen] = useState(false), [converting, setConverting] = useState(false);
  const active = useRef(true);
  useLayoutEffect(() => {active.current = true; return () => {active.current = false;};}, []);
  const block = editor.getBlock(blockId);
  const convertible = !!block && textTypes.has(block.type as typeof conversions[number]["type"]);
  const hasEmbeddedContent = Array.isArray(block?.content) && block.content.some(part => part.type !== "text");
  const availableConversions = conversions.filter(item => item.type in editor.schema.blockSchema && !(item.type === "codeBlock" && hasEmbeddedContent));
  const close = () => {if (!active.current) return; setConverting(false); setOpen(false); setMenuOpen(false);};
  const choose = (item: typeof conversions[number]) => {
    const current = editor.getBlock(blockId);
    if (!editor.isEditable || !current || !textTypes.has(current.type as typeof conversions[number]["type"])) return;
    // Code is plain text. Keep embedded references/formulas in a rich text block.
    if (item.type === "codeBlock" && Array.isArray(current.content) && current.content.some(part => part.type !== "text")) return;
    editor.transact(transaction => {
      closeHistory(transaction);
      if (item.type === "heading") editor.updateBlock(current, {type: "heading", props: item.props});
      else editor.updateBlock(current, {type: item.type});
    });
    editor.transact(transaction => {closeHistory(transaction);});
    close(); editor.focus();
  };
  const copyLink = async () => {
    if (!sourcePath || !editor.getBlock(blockId)) return;
    try {
      await navigator.clipboard.writeText(formatSourceLink({path: sourcePath, locator: {kind: "block", blockId}}));
      notify.success("已复制块链接"); close();
    } catch (reason) {notify.error(`复制块链接失败：${reason instanceof Error ? reason.message : String(reason)}`);}
  };
  return <ActionMenu label="拖动块；点击打开块菜单" menuLabel="块操作" className="note-block-actions"
    trigger={null} triggerRender={trigger} triggerClassName="bn-button note-block-drag-handle reorderable-drag-handle"
    contentClassName="note-block-action-menu" showTitle={false} portal placement="right-start" open={open}
    setOpen={value => {setOpen(value); if (!value) setConverting(false); setMenuOpen(value);}}>
    {convertible && editor.isEditable && availableConversions.length > 0 && <ActionMenu label="转换为" menuLabel="转换块类型" className="note-block-convert" triggerClassName="note-block-action-item"
      contentClassName="note-block-action-menu note-block-conversion-menu" portal
      open={converting} setOpen={setConverting} showTitle={false}
      trigger={<><ArrowRightLeft aria-hidden="true"/><span>转换为</span><ChevronRight aria-hidden="true"/></>}>
      {availableConversions.map(item => {
        const selected = block?.type === item.type && Object.entries(item.props).every(([key, value]) => block.props[key as keyof typeof block.props] === value);
        return <ActionMenuItem className="note-block-action-item" key={item.title} role="menuitemradio" aria-checked={selected}
          onClick={() => choose(item)}>
          <item.Icon aria-hidden="true"/><span>{item.title}</span>{selected && <Check aria-hidden="true"/>}
        </ActionMenuItem>;
      })}
    </ActionMenu>}
    <ActionMenuItem className="note-block-action-item" disabled={!sourcePath || !block} hideOnClick={false} onClick={() => {void copyLink();}}>
      <Link2 aria-hidden="true"/><span>复制块链接</span>
    </ActionMenuItem>
  </ActionMenu>;
}
