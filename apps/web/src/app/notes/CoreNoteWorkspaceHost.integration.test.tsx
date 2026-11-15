// @vitest-environment happy-dom
import {act, createRef, useEffect, useSyncExternalStore, type ComponentProps} from "react";
import {createRoot} from "react-dom/client";
import {beforeEach, expect, it, vi} from "vitest";
import type {HbsSession} from "@research-assistant/core/hbs";
import type {NoteDocumentV1} from "@research-assistant/core/notes";
import type {WorkspaceClosePort} from "../../core/contracts/workbench";
import {CoreNoteWorkspaceHost} from "../../core/features/notes/workspace/CoreNoteWorkspaceHost";
import {CoreNoteWorkspace} from "../../core/features/notes/workspace/CoreNoteWorkspace";
import {NotesProvider} from "../../core/features/notes/NotesClient";
import {browserNotesGateway} from "../../core/features/notes/gateways/browserNotesGateway";
import {selectAppRuntime} from "../../core/runtime/appRuntime";
import {createTestNamespacedStorage} from "../../core/runtime/testing/createTestNamespacedStorage";
import {createPluginConfigurations} from "../../core/configuration/configuration";
import {PluginConfigurationProvider} from "../../core/configuration/ConfigurationProvider";
import {WorkspaceTabsProvider, WorkspaceTabBar} from "../../core/components/workspace-tabs/WorkspaceTabs";
import {useNoteDocumentSession} from "../../core/public/noteDocuments";
import {NotesWorkspace} from "@research-assistant/note-manager-plugin/workspace";
import {noteConfiguration} from "@research-assistant/note-manager-plugin/configuration";

type HbsEditorProps = ComponentProps<typeof import("../../core/components/hbs/HbsNoteEditor").HbsNoteEditor>;
type DocumentViewProps = ComponentProps<typeof import("../../core/features/notes/workspace/NoteDocumentView").NoteDocumentView>;
type PublicSession = ReturnType<typeof useNoteDocumentSession>;
const observed = vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  return {
    editors: new WeakMap<HTMLTextAreaElement, HbsSession<NoteDocumentV1>>(),
    mounts: 0, unmounts: 0, current: undefined as PublicSession | undefined,
    errors: [] as string[],
  };
});

// Keep the production Core/manager/session/retained trees. Only the complex
// editor surface is replaced; edits and history replay use the real HBS owner.
vi.mock("../../core/components/hbs/HbsNoteEditor", () => ({
  HbsNoteEditor: (props: HbsEditorProps) => {
    const snapshot = useSyncExternalStore(props.session.subscribe, props.session.getSnapshot);
    useEffect(() => { observed.mounts += 1; return () => { observed.unmounts += 1; }; }, []);
    return <textarea aria-label="Host regression editor" value={snapshot.content.title ?? ""}
      data-page-id={snapshot.content.pageId}
      ref={node => { if (node) observed.editors.set(node, props.session); }}
      onChange={event => props.session.edit({...snapshot.content, title: event.currentTarget.value})}/>;
  },
}));
vi.mock("../../core/features/notes/workspace/NoteDocumentView", () => ({
  NoteDocumentView: (props: DocumentViewProps) => <section data-document-page={props.snapshot.page.id}>
    <button data-save-note onClick={props.onRetrySave}>保存</button>
  </section>,
}));
vi.mock("../../core/services/notifications", () => ({notify: {
  error: (message: string) => observed.errors.push(message), info: () => {},
}}));

let fixtureIndex = 0;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  observed.editors = new WeakMap(); observed.mounts = 0; observed.unmounts = 0;
  observed.current = undefined; observed.errors = [];
});

function Probe() {
  observed.current = useNoteDocumentSession({onError: message => observed.errors.push(message), onNotice() {}, onLoaded() {}});
  return null;
}

function current(): PublicSession {
  if (!observed.current) throw new Error("The Core note host has not published its session");
  return observed.current;
}
function editorSession(node: HTMLTextAreaElement): HbsSession<NoteDocumentV1> {
  const session = observed.editors.get(node);
  if (!session) throw new Error("The retained editor has no observed HBS session");
  return session;
}
function editor(root: Element): HTMLTextAreaElement {
  const node = root.querySelector<HTMLTextAreaElement>('textarea[aria-label="Host regression editor"]');
  if (!node) throw new Error("The Core retained editor is not mounted");
  return node;
}

async function fixture() {
  const storage = createTestNamespacedStorage(localStorage, `core-note-host-regression:${++fixtureIndex}:`);
  selectAppRuntime({mode: "browser", storage, gateways: {}, dispose() {}});
  const prefix = "research-assistant:workspace.v1:";
  storage.setItem(prefix + "notes-explorer-open", "true");
  const configuration = createPluginConfigurations(storage, [{
    owner: "note_manager", declaration: noteConfiguration, storage: {
      mode: {application: prefix + "notes-mode"}, scope: {application: prefix + "notes-scope"},
      explorerOpen: {application: prefix + "notes-explorer-open"}, fullIndex: {application: prefix + "notes-full-index"},
    },
  }]).get("note_manager");
  if (!configuration) throw new Error("The existing note configuration was not bound");
  const created = await browserNotesGateway.createPage({title: "Host identity note"});
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), close = createRef<WorkspaceClosePort>();
  const render = (mode: "core" | "manager" | "empty", floating = false) =>
    <NotesProvider gateway={browserNotesGateway}><WorkspaceTabsProvider>
      <CoreNoteWorkspaceHost initialPageId={created.page.id} embedded={false} active ref={close}>
        <Probe/>
        <div data-main-note>
          {mode === "manager" ? <PluginConfigurationProvider value={configuration}><NotesWorkspace active/></PluginConfigurationProvider>
            : mode === "core" ? <CoreNoteWorkspace embedded={false} active/> : null}
        </div>
        {floating && <div data-floating-note><CoreNoteWorkspace initialPageId={created.page.id} embedded active/></div>}
      </CoreNoteWorkspaceHost>
      <WorkspaceTabBar owner="notes" textActive={false} navigate={(_owner, _destination, ready) => { ready(); return true; }}/>
    </WorkspaceTabsProvider></NotesProvider>;
  const show = async (mode: "core" | "manager" | "empty", floating = false) => {
    await act(async () => root.render(render(mode, floating)));
  };
  const dispose = async () => { await act(async () => root.unmount()); host.remove(); storage.clear(); vi.unstubAllGlobals(); };
  return {host, created, close, show, dispose};
}

it("retains the same Core owner, HBS history and editor DOM through real management view replacement", async () => {
  const state = await fixture();
  try {
    await state.show("core");
    await vi.waitFor(() => expect(current().snapshot?.page.id).toBe(state.created.page.id));
    const node = editor(state.host), hbs = editorSession(node), owner = current().saveControllerRef.current;
    const editorId = current().editorSessionId;
    expect(owner).toBeDefined();
    const before = hbs.getSnapshot().content;
    let entry: ReturnType<typeof hbs.perform> | undefined;
    await act(async () => { entry = hbs.perform(() => hbs.edit({...before, title: "Input before management activates"})); });
    expect(node.value).toBe("Input before management activates");

    await state.show("manager");
    await vi.waitFor(() => expect(state.host.querySelector(".notes-explorer")).not.toBeNull());
    expect(current().saveControllerRef.current).toBe(owner);
    expect(current().editorSessionId).toBe(editorId);
    expect(editor(state.host)).toBe(node);
    expect(editorSession(node)).toBe(hbs);
    expect(node.isConnected).toBe(true);
    expect(state.host.querySelectorAll(".retained-note-editor")).toHaveLength(1);
    expect(state.host.querySelectorAll(`[data-tab-id="notes:${state.created.page.id}"]`)).toHaveLength(1);
    expect(observed.mounts).toBe(1);
    expect(observed.unmounts).toBe(0);

    if (!entry) throw new Error("The original HBS history entry was not created");
    const history = entry;
    await act(async () => hbs.restore(history, "before"));
    expect(node.value).toBe(before.title ?? "");
    await act(async () => hbs.restore(history, "after"));
    expect(node.value).toBe("Input before management activates");
    await act(async () => state.host.querySelector<HTMLButtonElement>("[data-save-note]")?.click());
    await vi.waitFor(async () => expect((await browserNotesGateway.loadDocument(state.created.page.id)).page.title)
      .toBe("Input before management activates"));

    await state.show("core");
    expect(current().saveControllerRef.current).toBe(owner);
    expect(current().snapshot?.page.id).toBe(state.created.page.id);
    expect(editor(state.host)).toBe(node);
    expect(node.value).toBe("Input before management activates");
    expect(observed.mounts).toBe(1);
    expect(observed.unmounts).toBe(0);
    expect(await browserNotesGateway.listRevisions(state.created.page.id)).not.toHaveLength(0);

    await state.show("empty");
    expect(node.isConnected).toBe(false);
    expect(current().saveControllerRef.current).toBe(owner);
    expect(observed.unmounts).toBe(0);
    await state.show("core");
    expect(editor(state.host)).toBe(node);
    expect(editorSession(node)).toBe(hbs);
    expect(node.isConnected).toBe(true);
    expect(observed.mounts).toBe(1);
  } finally { await state.dispose(); }
});

it("keeps the shared document mounted when its existing close gate refuses preservation", async () => {
  const state = await fixture();
  try {
    await state.show("core");
    await vi.waitFor(() => expect(current().snapshot?.page.id).toBe(state.created.page.id));
    const node = editor(state.host), owner = current().saveControllerRef.current;
    if (!owner || !state.close.current) throw new Error("The Core close/save owner is not ready");
    const failure = vi.spyOn(owner, "flush").mockRejectedValueOnce(new Error("MODEL_PRESERVATION_FAILED: synthetic host gate"));
    await act(async () => expect(state.close.current?.prepareClose()).resolves.toBe(false));
    expect(editor(state.host)).toBe(node);
    expect(node.isConnected).toBe(true);
    expect(current().saveControllerRef.current).toBe(owner);
    expect(observed.errors.join("\n")).toContain("MODEL_PRESERVATION_FAILED");
    failure.mockRestore();
    await act(async () => expect(state.close.current?.prepareClose()).resolves.toBe(true));
  } finally { await state.dispose(); }
});

it("retains the main scope while an embedded same-page note owns an independent HBS scope", async () => {
  const state = await fixture();
  try {
    await state.show("core");
    await vi.waitFor(() => expect(current().snapshot?.page.id).toBe(state.created.page.id));
    const mainNode = editor(state.host), main = editorSession(mainNode), owner = current().saveControllerRef.current;
    await state.show("core", true);
    await vi.waitFor(() => expect(state.host.querySelectorAll('textarea[aria-label="Host regression editor"]')).toHaveLength(2));
    const floatingHost = state.host.querySelector("[data-floating-note]");
    if (!floatingHost) throw new Error("The embedded note did not mount");
    const floatingNode = editor(floatingHost), floating = editorSession(floatingNode);
    expect(floating).not.toBe(main);
    expect(current().saveControllerRef.current).toBe(owner);
    expect(editor(state.host)).toBe(mainNode);
    expect(main.getSnapshot().content.pageId).toBe(floating.getSnapshot().content.pageId);
    await state.show("core");
    expect(editor(state.host)).toBe(mainNode);
    expect(current().saveControllerRef.current).toBe(owner);
    expect(mainNode.isConnected).toBe(true);
    expect(floatingNode.isConnected).toBe(false);
  } finally { await state.dispose(); }
});
