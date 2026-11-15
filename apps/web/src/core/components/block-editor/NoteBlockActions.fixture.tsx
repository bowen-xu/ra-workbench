import {StrictMode} from "react";
import {createRoot} from "react-dom/client";
import {useCreateBlockNote} from "@blocknote/react";
import {NoteEditorSurface, baseBlockEditorSchema, blockEditorDefaults} from "./index";

/** Acceptance data only; both hosts render the production editor and menus. */
export function mountNoteBlockActionsFixture(): void {
  function Host({name, path}: {name: string; path: readonly string[]}) {
    const editor = useCreateBlockNote({...blockEditorDefaults, schema: baseBlockEditorSchema, initialContent: [
      {id: "table", type: "table", content: {type: "tableContent", rows: [
        {cells: ["A", "B"]}, {cells: ["C", "D"]},
      ]}},
      {id: "tail", type: "paragraph", content: "Outside menu"},
    ]}) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
    return <section aria-label={name} style={{minWidth: 0, background: "white"}}>
      <h2>{name}</h2>
      <NoteEditorSurface editor={editor} blockSourcePath={path} onChange={() => undefined}/>
    </section>;
  }
  const target = document.createElement("div");
  Object.assign(target.style, {position: "fixed", inset: "0", display: "grid", gridTemplateColumns: "1fr 1fr", gap: "24px", padding: "32px", background: "#eee"});
  document.body.replaceChildren(target);
  createRoot(target).render(<StrictMode>
    <Host name="普通笔记" path={["notes", "fixture"]}/>
    <Host name="写作笔记" path={["writings", "fixture", "notes", "idea.ranote"]}/>
  </StrictMode>);
}
