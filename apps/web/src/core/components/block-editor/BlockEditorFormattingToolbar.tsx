import { NOTE_BLOCK_TYPES } from "./noteBlockTypes";
import { clearBlockEditorFormatting } from "./clearBlockEditorFormatting";
import { ToolbarItem } from "@ariakit/react";
import { COLORS_DEFAULT } from "@blocknote/core";
import { FormattingToolbar, FormattingToolbarController, getFormattingToolbarItems, useBlockNoteEditor,
  useEditorState, type FormattingToolbarProps } from "@blocknote/react";
import { RemoveFormatting } from "lucide-react";
import { createContext, useContext, useLayoutEffect, useRef, useState, type ComponentProps, type CSSProperties, type ReactNode } from "react";
import { ControlHint } from "../ControlHint";
import { ActionMenu, ActionMenuItem } from "../ActionMenu";
import "./BlockEditorFormattingToolbar.css";
import { MarkSelectionAsFormulaButton } from "./MarkSelectionAsFormula";
import { BlockSetSelection } from "./BlockSetSelection";
import {NodeSelection} from "@tiptap/pm/state";
import {isFixedBlock} from "./blockEditingConstraints";
import {NOTE_HEADING_LEVELS} from "./noteHeading";
import {useSelectionToolbarVisibility} from "./useSelectionToolbarVisibility";

/** The text toolbar assumes a contiguous text selection. Structural selections
 * retain block-menu commands instead of presenting an incorrect text range.
 */
export function BlockEditorFormattingToolbarController(props: ComponentProps<typeof FormattingToolbarController>): React.JSX.Element | null {
  const editor = useBlockNoteEditor();
  const range = useEditorState({editor, selector: ({editor}) => ({
    from: editor.prosemirrorState.selection.from, to: editor.prosemirrorState.selection.to,
  })});
  const visible = useSelectionToolbarVisibility(editor, range.from, range.to);
  const structural = useEditorState({ editor, selector: ({ editor }) => editor.prosemirrorState.selection instanceof BlockSetSelection
    || editor.prosemirrorState.selection instanceof NodeSelection
    || isFixedBlock({type: editor.prosemirrorState.selection.$from.parent.type.name})
    || isFixedBlock({type: editor.prosemirrorState.selection.$to.parent.type.name}) });
  // Keep the editor's selection/extension state intact while hiding the UI.
  // On remount BlockNote reads the current selection, never a saved old range.
  // Preserve its offset/shift/flip and the shared editor overlay portal.
  return structural || !visible ? null : <FormattingToolbarController {...props} floatingUIOptions={{
    ...props.floatingUIOptions,
    useFloatingOptions: {...props.floatingUIOptions?.useFloatingOptions, placement: "bottom-start"},
  }} />;
}

const colors = ["default", "gray", "brown", "orange", "yellow", "green", "blue", "purple", "pink", "red"] as const;
type Color = typeof colors[number];
type ColorKind = "textColor" | "backgroundColor";
interface ColorChoice { readonly kind: ColorKind; readonly color: Color; }
const names: Record<Color, string> = { default: "自动", gray: "灰色", brown: "棕色", orange: "橙色", yellow: "黄色",
  green: "绿色", blue: "蓝色", purple: "紫色", pink: "粉色", red: "红色" };
const historyContext = createContext<{ readonly recent: readonly ColorChoice[]; readonly remember: (choice: ColorChoice) => void } | undefined>(undefined);

/** Recent choices belong to the host editor session, not research data. */
export function BlockColorHistory({ children }: { readonly children: ReactNode }): React.JSX.Element {
  const [recent, setRecent] = useState<readonly ColorChoice[]>([]);
  return <historyContext.Provider value={{ recent, remember: choice => setRecent(previous => [choice,
    ...previous.filter(item => item.kind !== choice.kind || item.color !== choice.color)].slice(0, 5)) }}>{children}</historyContext.Provider>;
}

function ColorStyleMenu(): React.JSX.Element | null {
  const editor = useBlockNoteEditor();
  const history = useContext(historyContext)!;
  const [open, setOpen] = useState(false);
  const focusAfterCommand = useRef(false);
  useLayoutEffect(() => {
    if (open || !focusAfterCommand.current) return;
    focusAfterCommand.current = false;
    const active = editor.domElement?.ownerDocument.activeElement;
    // Complete the command after the closed menu has committed and Ariakit's
    // descendant dismissal effects have run. Focusing during click/on-hide can
    // still race an expanded disclosure and the popup's native focus owner.
    // Do not steal focus if the command callback opened another control.
    if (active && active !== active.ownerDocument.body && active !== editor.domElement
      && !active.closest(".block-color-menu, .block-color-control")) return;
    editor.focus();
  }, [editor, open]);
  const state = useEditorState({ editor, selector: ({ editor }) => {
    const blocks = editor.getSelection()?.blocks ?? [editor.getTextCursorPosition().block];
    if (!editor.isEditable || !blocks.some(block => block.content !== undefined)) return undefined;
    const styles = editor.getActiveStyles();
    return { textColor: styles.textColor ?? "default", backgroundColor: styles.backgroundColor ?? "default" };
  } });
  if (!state) return null;
  const choose = (choice: ColorChoice) => {
    focusAfterCommand.current = true;
    if (choice.color === "default") editor.removeStyles({ [choice.kind]: "default" });
    else editor.addStyles({ [choice.kind]: choice.color });
    history.remember(choice);
  };
  const swatch = (choice: ColorChoice, recent = false) => {
    const text = choice.color === "default" ? "#2f2c27" : COLORS_DEFAULT[choice.color]!.text;
    const background = choice.color === "default" ? "#fff" : COLORS_DEFAULT[choice.color]!.background;
    const kindName = choice.kind === "textColor" ? "文本颜色" : "背景颜色";
    return <ActionMenuItem key={`${choice.kind}-${choice.color}`} role="menuitemradio"
      aria-label={`${recent ? "最近使用 · " : ""}${kindName} · ${names[choice.color]}`}
      aria-checked={state[choice.kind] === choice.color} onClick={() => choose(choice)}
      className={`block-color-swatch ${choice.kind === "textColor" ? "text" : "background"}`}
      style={{ "--swatch-text": text, "--swatch-background": background } as CSSProperties}>
      {choice.kind === "textColor" && <span aria-hidden="true">A</span>}
    </ActionMenuItem>;
  };
  return <ActionMenu label="颜色" menuLabel="文本与背景颜色" hint portal open={open} setOpen={setOpen}
    autoFocusOnHide={element => {
      if (focusAfterCommand.current) return false;
      // Escape returns to the trigger, but a later style command or an outside
      // control owns any focus it has already received during menu dismissal.
      const active = element?.ownerDocument.activeElement;
      if (active && active !== document.body && active !== element && !active.closest(".block-color-menu")) return false;
      element?.focus({ preventScroll: true });
      return false;
    }}
    className="block-color-control" triggerRender={<ToolbarItem />} portalElement={editor.portalElement}
    triggerClassName="bn-ak-button bn-ak-secondary block-color-trigger"
    contentClassName="block-color-menu" trigger={<span className="block-color-trigger-icon" aria-hidden="true">A</span>}>
    {history.recent.length > 0 && <section aria-label="最近使用"><h3>最近使用</h3>
      <div className="block-color-grid">{history.recent.map(choice => swatch(choice, true))}</div></section>}
    <section aria-label="文本颜色"><h3>文本颜色</h3>
      <div className="block-color-grid">{colors.map(color => swatch({ kind: "textColor", color }))}</div></section>
    <section aria-label="背景颜色"><h3>背景颜色</h3>
      <div className="block-color-grid">{colors.map(color => swatch({ kind: "backgroundColor", color }))}</div></section>
  </ActionMenu>;
}

function ClearFormattingButton(): React.JSX.Element | null {
  const editor = useBlockNoteEditor();
  const enabled = useEditorState({ editor, selector: ({ editor }) => editor.isEditable
    && (editor.getSelection()?.blocks ?? [editor.getTextCursorPosition().block]).some(block => block.content !== undefined) });
  if (!enabled) return null;
  return <ControlHint content="清除样式"><ToolbarItem aria-label="清除样式"
    className="bn-ak-button bn-ak-secondary bn-button" onMouseDown={event => event.preventDefault()} onClick={() => {
      clearBlockEditorFormatting(editor);
    }}><RemoveFormatting /></ToolbarItem></ControlHint>;
}

export function BlockEditorFormattingToolbar({ children, leadingControls, ...props }: FormattingToolbarProps & { readonly children?: ReactNode; readonly leadingControls?: ReactNode }): React.JSX.Element {
  // BlockNote 0.52 checks prop types here, not allowed values; schema levels alone
  // do not remove H5/H6 from its default block-type selector.
  const candidates: NonNullable<FormattingToolbarProps["blockTypeSelectItems"]> = props.blockTypeSelectItems
    ?? NOTE_BLOCK_TYPES.filter(item => item.type !== "codeBlock").map(item => ({name: item.title, type: item.type, props: item.props, icon: item.Icon}));
  const types = candidates.filter(item => item.type !== "heading" || NOTE_HEADING_LEVELS.some(level => level === item.props?.level));
  return <FormattingToolbar {...props} blockTypeSelectItems={types}>{leadingControls}{getFormattingToolbarItems(types).flatMap(item => {
    if (item.key === "blockTypeSelect") return [item, <ColorStyleMenu key="colors" />];
    if (item.key === "colorStyleButton") return [];
    if (item.key === "strikeStyleButton") return [item, <ClearFormattingButton key="clearFormatting" />];
    return [item];
  })}{children}<MarkSelectionAsFormulaButton /></FormattingToolbar>;
}
