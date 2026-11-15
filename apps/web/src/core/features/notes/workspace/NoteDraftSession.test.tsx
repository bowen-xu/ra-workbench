// @vitest-environment happy-dom
import {act,StrictMode} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {NotesClient} from "../NotesClient";
import {browserNotesGateway} from "../gateways/browserNotesGateway";
import {selectAppRuntime} from "../../../runtime/appRuntime";
import * as textSessions from "../editor/noteTextEditorSession";
import {useNoteDocumentSession} from "./useNoteDocumentSession";
import {isAbandonedEmptyNoteDraft} from "./noteDraftIdentity";

it("registers canonical tool ports on first open and retains the editor owner through its first edit",async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  localStorage.clear();selectAppRuntime({mode:"browser",storage:localStorage,gateways:{},dispose(){}});
  const register=vi.spyOn(textSessions,"registerNoteTextSession").mockReturnValue(()=>{});
  const client=new NotesClient(browserNotesGateway),draft=await client.creation.create();
  expect(draft.page.id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
  expect(isAbandonedEmptyNoteDraft(draft.page.id,[])).toBe(false);
  const options={client,onError:vi.fn(),onNotice:vi.fn(),onLoaded:vi.fn()};
  let page!:ReturnType<typeof useNoteDocumentSession>;
  function Harness(){page=useNoteDocumentSession(options);return null;}
  const root=createRoot(document.createElement("div"));
  try {
    await act(async()=>root.render(<StrictMode><Harness/></StrictMode>));
    await act(async()=>page.loadPage(draft.page.id));
    const controller=page.saveControllerRef.current,editorId=page.editorSessionId;
    expect(register).toHaveBeenCalledTimes(1);
    await act(async()=>{
      page.onDocumentChange({...draft.document,title:"First canonical title"});
      await controller?.flush();
    });
    expect(options.onError).not.toHaveBeenCalled();
    expect(register).toHaveBeenCalledTimes(1);
    expect(page.saveControllerRef.current).toBe(controller);
    expect(page.editorSessionId).toBe(editorId);
    await act(async()=>expect(await page.ensureDurable(true)).toBe(true));
    expect((await browserNotesGateway.loadDocument(draft.page.id)).page.title).toBe("First canonical title");
    expect(isAbandonedEmptyNoteDraft(draft.page.id,[])).toBe(false);
  } finally {
    await act(async()=>root.unmount());register.mockRestore();vi.unstubAllGlobals();
  }
});
