// @vitest-environment happy-dom
import {act, useEffect} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import type {ReadingNoteBlock, NotePageId} from "@research-assistant/core/notes";
import type {NotesGateway} from "@research-assistant/data/notes";
import {NotesProvider, useNotesClient, type NotesClient} from "../../../../../../../apps/web/src/core/features/notes/NotesClient";
import {AnnotationController} from "../../../../../../../../plugins/ra-literature-library/src/features/reader/annotations/AnnotationController";
import {useAnnotationRefreshVersion} from "../../../../../../../../plugins/ra-literature-library/src/features/reader/annotations/useAnnotationRefreshVersion";

function note(id: string, attachmentId: string): ReadingNoteBlock {
  return {id:id as ReadingNoteBlock["id"],pageId:id as NotePageId,noteType:"annotation",body:"Saved",color:"#f4cf65",
    status:"active",isFavorite:false,revision:0,projectIds:[],tagIds:[],createdAt:"now",updatedAt:"now",
    source:{anchorId:id,paperId:"paper",paperTitle:"Paper",attachmentId,contentSha256:"a".repeat(64),pageIndex:0,
      rects:[],quoteExact:"quote",quotePrefix:"",quoteSuffix:"",state:"exact"}};
}

it("defers hidden refreshes and ignores unrelated documents across retained PDF controllers", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const rows = [note("first", "pdf-1"), note("second", "pdf-2")];
  const list = vi.fn(async (query: {status: string}) => query.status === "active" ? rows : []);
  const gateway = {listReadingNotes:list} as unknown as NotesGateway;
  const controllers = [0,1].map(() => new AnnotationController({gateway,onCommitted:()=>undefined}));
  const host = document.createElement("div"), root = createRoot(host);
  let client!: NotesClient;
  function Reader({index, active}: {index:number;active:boolean}) {
    client = useNotesClient();
    const controller = controllers[index]!;
    const refresh = useAnnotationRefreshVersion(controller, active);
    useEffect(() => {void controller.load("paper", `pdf-${index+1}`);}, [controller, refresh]);
    return null;
  }
  const render = (first: boolean, second: boolean) => act(async () => root.render(<NotesProvider gateway={gateway}>
    <Reader index={0} active={first}/><Reader index={1} active={second}/>
  </NotesProvider>));
  const changed = (pageId: string, revision: number) => act(async () => client.notifyChanged({kind:"document",pageId:pageId as NotePageId,revision}));
  try {
    await render(true, false);
    expect(list).toHaveBeenCalledTimes(4); // First mount retains the original recovery reads.
    list.mockClear();
    await changed("ordinary-note", 1);
    expect(list).not.toHaveBeenCalled();
    await changed("second", 1); await changed("second", 2);
    expect(list).not.toHaveBeenCalled();
    await render(false, true);
    expect(list).toHaveBeenCalledTimes(2);
    list.mockClear();
    await render(true, false); await render(false, true);
    expect(list).not.toHaveBeenCalled();
    await render(false, false);
    await act(async () => client.notifyChanged());
    expect(list).not.toHaveBeenCalled();
    await render(true, false);
    expect(list).toHaveBeenCalledTimes(2);
    list.mockClear();
    await changed("first", 3);
    expect(list).toHaveBeenCalledTimes(2);
    list.mockClear();
    await render(false, true);
    expect(list).toHaveBeenCalledTimes(2); // Broad invalidation remained pending for the second PDF.
    list.mockClear();
    await act(async () => {
      for (let i=0;i<70;i++) client.notifyChanged({kind:"document",pageId:`unrelated-${i}` as NotePageId,revision:1});
    });
    // A missing invalidation history is conservatively rechecked, never treated
    // as proof that a retained reader is current.
    expect(list).toHaveBeenCalledTimes(2);
  } finally {
    await act(async () => root.unmount());
    for (const controller of controllers) controller.dispose();
    host.remove(); vi.unstubAllGlobals();
  }
});
