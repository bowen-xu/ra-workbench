// @vitest-environment happy-dom
import {act, type ComponentProps} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {CoreNoteWorkspace} from "./CoreNoteWorkspace";
import {NotesProvider} from "../NotesClient";
import {browserNotesGateway} from "../gateways/browserNotesGateway";
import {selectAppRuntime} from "../../../runtime/appRuntime";

const rendered = vi.hoisted(() => ({view: undefined as ComponentProps<typeof import("./NoteDocumentView").NoteDocumentView> | undefined,
  editors: undefined as ComponentProps<typeof import("./RetainedNoteEditors").RetainedNoteEditors> | undefined}));
vi.mock("./NoteDocumentView", () => ({NoteDocumentView: (props: NonNullable<typeof rendered.view>) => {
  rendered.view = props; return <button onClick={props.onRetrySave}>保存 {props.snapshot.page.id}</button>;
}}));
vi.mock("./RetainedNoteEditors", async original => ({
  ...await original<typeof import("./RetainedNoteEditors")>(),
  OwnedRetainedNoteEditors: (props: NonNullable<typeof rendered.editors>) => {rendered.editors = props; return null;},
}));

it("opens, edits, saves, and reopens the canonical Core note with no management plugin or domain gateway", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear();
  const runtime = {mode: "browser" as const, storage: localStorage, gateways: {}, dispose() {}};
  selectAppRuntime(runtime);
  const created = await browserNotesGateway.createPage({title: "Existing Core document"});
  const page = created.page;
  localStorage.setItem("research-assistant:workspace.v1:notes-mode", '"dependent"');
  const host = document.createElement("div"); document.body.append(host); let root = createRoot(host);
  const render = () => <NotesProvider gateway={browserNotesGateway}><CoreNoteWorkspace initialPageId={page.id}/></NotesProvider>;
  try {
    await act(async () => root.render(render()));
    await vi.waitFor(() => expect(rendered.view?.snapshot.page.id).toBe(page.id));
    const before = rendered.view!.snapshot, session = rendered.view!.editorSessionId;
    await act(async () => {
      rendered.editors!.onChange({...before.document, title: "Saved through Core"}, session);
      rendered.view!.onRetrySave();
    });
    await vi.waitFor(async () => expect((await browserNotesGateway.loadDocument(page.id)).page.title).toBe("Saved through Core"));
    expect(await browserNotesGateway.listRevisions(page.id)).not.toHaveLength(0);
    expect(localStorage.getItem("research-assistant:workspace.v1:notes-mode")).toBe('"dependent"');
    expect(runtime.gateways).toEqual({});
    await act(async () => root.unmount()); root = createRoot(host);
    await act(async () => root.render(render()));
    await vi.waitFor(() => expect(rendered.view?.snapshot.page.title).toBe("Saved through Core"));
    expect(rendered.view?.snapshot.page.id).toBe(page.id);
  } finally {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();}
});
