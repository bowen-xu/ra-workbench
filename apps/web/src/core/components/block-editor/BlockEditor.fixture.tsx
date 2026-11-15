import * as React from "react";
import { createRoot } from "react-dom/client";
import { useCreateBlockNote } from "@blocknote/react";
import { NoteEditorSurface, baseBlockEditorSchema, blockEditorDefaults } from "./index";
import { EditorSuggestionMenu } from "./EditorSuggestionMenu";

/** Browser-only two-host acceptance fixture, not imported by the application. */
export function mountSharedEditorFixture(): void {
    // No NotesClient, page ID, gateway, or NotesWorkspace CSS in either host.
    function Host({ name }: { name: string }) {
      const [revision, setRevision] = React.useState(0);
      const [readOnly, setReadOnly] = React.useState(false);
      const [saved, setSaved] = React.useState("");
      const editor = useCreateBlockNote({ ...blockEditorDefaults, schema: baseBlockEditorSchema, initialContent: [
        { id: "first", type: "paragraph", content: "First" },
        { id: "parent", type: "paragraph", content: "Parent", children: [{ id: "child", type: "paragraph", content: "Child" }] },
        { id: "tail", type: "paragraph", content: "Tail" },
        { id: "formula", type: "formulaBlock", props: { source: "x^2" } },
      ] }) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
      return <section aria-label={name} style={{ minWidth: 0, overflow: "auto", height: 580, background: "white" }}>
        <h2>{name}</h2>
        <button onClick={() => setRevision(revision + 1)}>保存回执</button>
        <button onClick={() => window.setTimeout(() => setRevision(value => value + 1), 1000)}>延迟保存回执</button>
        <button onClick={() => setReadOnly(!readOnly)}>切换只读</button>
        <output aria-label="宿主保存内容" data-revision={revision}>{saved}</output>
        <NoteEditorSurface editor={editor} readOnly={readOnly} movePolicy={{ maxDepth: 64 }} onChange={() => {
          // Deliberately different host encodings: neither creates a Note Page.
          setSaved(name === "写作思路" ? JSON.stringify({ outline: editor.document })
            : editor.document.map((block: { id: string }) => block.id).join(","));
        }}>
          {!readOnly && <EditorSuggestionMenu triggerCharacter="@" getItems={async () => [{ title: "宿主扩展", onItemClick: () => undefined }]} />}
        </NoteEditorSurface>
      </section>;
    }
    const target = document.createElement("div");
    Object.assign(target.style, { position: "fixed", inset: "0", display: "grid", gridTemplateColumns: "1fr 1fr", gap: "24px", padding: "16px", background: "#eee" });
    document.body.replaceChildren(target);
    createRoot(target).render(React.createElement(React.Fragment, null,
      React.createElement(Host, { name: "写作思路" }), React.createElement(Host, { name: "临时草稿" })));
    const style = document.createElement("style");
    style.textContent = "output { display: none; }";
    document.head.append(style);
}
