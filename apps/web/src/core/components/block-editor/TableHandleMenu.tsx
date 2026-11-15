import {TableHandle, useBlockNoteEditor, useEditorState, useExtension, useExtensionState, type TableHandleProps} from "@blocknote/react";
import {TableHandlesExtension} from "@blocknote/core/extensions";
import type {DefaultBlockSchema,DefaultInlineContentSchema,DefaultStyleSchema} from "@blocknote/core";
import {closeHistory} from "@tiptap/pm/history";
import {ArrowDown,ArrowLeft,ArrowRight,ArrowUp,Trash2} from "lucide-react";
import {Menu} from "../overlay/OverlayPopup";
import {NoteActionMenuItem} from "./NoteActionMenu";
import {SwitchIndicator} from "../Switch";

/** Keep upstream handle geometry, freezing, focus return and row/column drag.
 * Only the menu is composed here; native commands own table transactions. */
export function NoteTableHandle(props: TableHandleProps): React.JSX.Element {
  return <TableHandle {...props} tableHandleMenu={props.orientation==="row" ? RowMenu : ColumnMenu}/>;
}
const RowMenu=()=> <NoteTableHandleMenu orientation="row"/>;
const ColumnMenu=()=> <NoteTableHandleMenu orientation="column"/>;

export function NoteTableHandleMenu({orientation}: {readonly orientation: "row" | "column"}): React.JSX.Element | null {
  const editor=useBlockNoteEditor<{table:DefaultBlockSchema["table"]},DefaultInlineContentSchema,DefaultStyleSchema>(),handles=useExtension(TableHandlesExtension);
  const state=useExtensionState(TableHandlesExtension);
  // Handles are frozen while a menu is open. Read the live block so repeated
  // toggles and Undo reflect current content rather than the hover snapshot.
  const block=useEditorState({editor,selector:()=>state?.block.id ? editor.getBlock(state.block.id) : undefined});
  if(!handles || !state || block?.type!=="table")return null;
  const index=orientation==="row" ? state.rowIndex : state.colIndex;
  if(index===undefined)return null;
  const isRow=orientation==="row",key=isRow ? "headerRows" : "headerCols",checked=!!block.content[key];
  const toggleHeader=()=>{
    const current=editor.getBlock(block.id);
    if(!editor.isEditable || current?.type!=="table")return;
    editor.transact(tr=>{
      closeHistory(tr);
      editor.updateBlock(current,{content:{...current.content,[key]:current.content[key] ? undefined : 1}});
    });
    editor.transact(tr=>{closeHistory(tr);});
  };
  const before=()=>{if(editor.isEditable)handles.addRowOrColumn(index,isRow ? {orientation:"row",side:"above"} : {orientation:"column",side:"left"});};
  const after=()=>{if(editor.isEditable)handles.addRowOrColumn(index,isRow ? {orientation:"row",side:"below"} : {orientation:"column",side:"right"});};
  const BeforeIcon=isRow ? ArrowUp : ArrowLeft,AfterIcon=isRow ? ArrowDown : ArrowRight;
  return <Menu className="note-action-menu note-table-handle-menu" aria-label={isRow ? "表格行操作" : "表格列操作"}
    gutter={6} modal={false} unmountOnHide>
    {index===0 && editor.settings.tables.headers && <NoteActionMenuItem role="menuitemcheckbox" aria-checked={checked}
      disabled={!editor.isEditable} hideOnClick={false} onClick={toggleHeader}>
      <span>{isRow ? "表头行" : "表头列"}</span><SwitchIndicator checked={checked}/>
    </NoteActionMenuItem>}
    <NoteActionMenuItem disabled={!editor.isEditable} onClick={()=>{if(editor.isEditable)handles.removeRowOrColumn(index,orientation);}}>
      <Trash2 aria-hidden="true"/><span>{isRow ? "删除行" : "删除列"}</span>
    </NoteActionMenuItem>
    <NoteActionMenuItem disabled={!editor.isEditable} onClick={before}><BeforeIcon aria-hidden="true"/><span>{isRow ? "在上方插入行" : "在左侧插入列"}</span></NoteActionMenuItem>
    <NoteActionMenuItem disabled={!editor.isEditable} onClick={after}><AfterIcon aria-hidden="true"/><span>{isRow ? "在下方插入行" : "在右侧插入列"}</span></NoteActionMenuItem>
  </Menu>;
}
