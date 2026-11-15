import type { DefaultReactSuggestionItem } from "@blocknote/react";
import { BlockEditor, type BlockEditorProps, type BlockEditorInstance } from "./BlockEditor";
import type { BlockTreeNode } from "./blockSelectionRoots";
import { SlashSuggestionMenu } from "./SynchronizedSuggestionMenu";
import { EditorSuggestionMenu } from "./EditorSuggestionMenu";
import { buildNoteEditingItems, type NoteEditingSource } from "./noteEditingCommands";
import { filterSuggestionItems } from "@blocknote/core/extensions";
import "./NoteEditorSurface.css";
import { EquationSuggestionMenu } from "./EquationSuggestionMenu";
import {TableMediaControls, type TableMediaEditor} from "./TableMediaControls";
import {NoteBlockActionsContext} from "./NoteBlockActions";
import {BlockSourceNavigationController, type BlockSourceNavigation} from "./BlockSourceNavigation";
import {useLayoutEffect, type MouseEvent} from "react";
import {notePasteFormatting} from "./notePasteFormatting";
import {CaptionBlockLayout} from "./CaptionBlockLayout";
import {ImageClipboard} from "./ImageClipboard";
import {PlainTextPaste} from "./PlainTextPaste";
import {noteToggleEditing} from "./noteToggleEditing";

interface Props<Block extends BlockTreeNode> extends Omit<BlockEditorProps<Block>, "defaultSlashMenu" | "editor"> {
  readonly editor: BlockEditorInstance<Block> & NoteEditingSource;
  readonly additionalSlashItems?: readonly DefaultReactSuggestionItem[];
  readonly blockSourcePath?: readonly string[];
  readonly sourceNavigation?: BlockSourceNavigation | undefined;
  readonly onOpenSourceLink?: (href: string) => void;
}

/** Shared note editing UI. Hosts inject their schema/session, persistence,
 * resource commands and optional review/navigation controls. */
export function NoteEditorSurface<Block extends BlockTreeNode>({ additionalSlashItems = [], blockSourcePath, sourceNavigation, onOpenSourceLink, onClickCapture, onAuxClickCapture, children, className, ...props }: Props<Block>): React.JSX.Element {
  useLayoutEffect(() => {
    const extension = notePasteFormatting();
    const toggleEditing = noteToggleEditing();
    props.editor.registerExtension(extension);
    props.editor.registerExtension(toggleEditing);
    return () => {
      props.editor.unregisterExtension("note-toggle-editing");
      props.editor.unregisterExtension(extension.key);
    };
  }, [props.editor]);
  const openSource = (event: MouseEvent<HTMLDivElement>) => {
    const anchor = event.target instanceof Element ? event.target.closest("a[href]") : null;
    const href = anchor?.getAttribute("href");
    if (!onOpenSourceLink || !href || !/^source:/iu.test(href)) return false;
    event.preventDefault(); event.stopPropagation(); onOpenSourceLink(href); return true;
  };
  return <NoteBlockActionsContext.Provider value={blockSourcePath ? {sourcePath: blockSourcePath} : {}}><BlockEditor {...props} className={`note-editor-surface${className ? ` ${className}` : ""}`} defaultSlashMenu={false}
    onClickCapture={event => {if (!openSource(event)) onClickCapture?.(event);}}
    onAuxClickCapture={event => {if (!openSource(event)) onAuxClickCapture?.(event);}}>
    {children}
    <BlockSourceNavigationController editor={props.editor} navigation={sourceNavigation}/>
    <TableMediaControls editor={props.editor as unknown as TableMediaEditor} />
    <CaptionBlockLayout editor={props.editor}/>
    <ImageClipboard editor={props.editor}/>
    <PlainTextPaste/>
    {!props.readOnly && <>
      <EquationSuggestionMenu />
      <EditorSuggestionMenu suggestionMenuComponent={SlashSuggestionMenu}
        getItems={async query => filterSuggestionItems([...buildNoteEditingItems(props.editor), ...additionalSlashItems], query)} triggerCharacter="/"
        floatingUIOptions={{ elementProps: { className: "note-slash-menu-popover" } }} />
    </>}
  </BlockEditor></NoteBlockActionsContext.Provider>;
}
