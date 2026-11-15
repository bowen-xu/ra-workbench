/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {EditorView} from "@codemirror/view";
import {TextDiffEditor} from "./TextDiffEditor";
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
afterEach(()=>{document.body.replaceChildren();vi.restoreAllMocks();});
async function mount(){const host=document.createElement("div");document.body.append(host);const root=createRoot(host);return{host,root,close:()=>act(async()=>root.unmount())};}
it("read-only documents use the existing merge widgets and exact Undo/Keep commands",async()=>{
 const view=await mount();const review=vi.fn();
 try{
  await act(async()=>view.root.render(<TextDiffEditor content={"new text\n"} baseline={"old text\n"} onReview={review}/>));
  expect(view.host.querySelector('[contenteditable="false"]')).not.toBeNull();
  expect(view.host.querySelector('.editor-diff-chunk-actions')).not.toBeNull();
  await act(async()=>view.host.querySelector<HTMLButtonElement>('[aria-label="Keep all"]')!.click());
  expect(review).toHaveBeenLastCalledWith({content:"new text\n",baseline:"new text\n"});
  await act(async()=>view.root.render(<TextDiffEditor content={"new text\n"} baseline={"old text\n"} syncVersion={1} onReview={review}/>));
  await act(async()=>view.host.querySelector<HTMLButtonElement>('[aria-label="撤销第 1 处变更"]')!.click());
  expect(review).toHaveBeenLastCalledWith({content:"old text\n",baseline:"old text\n"});
 }finally{await view.close();}
});
it("failed save can restore canonical diff; explicit editing preserves draft callbacks",async()=>{
 const view=await mount();const review=vi.fn(),change=vi.fn();
 try{
  await act(async()=>view.root.render(<TextDiffEditor content="new" baseline="old" editable onChange={change} onReview={review}/>));
  const cm=EditorView.findFromDOM(view.host.querySelector('.cm-editor')!)!;
  await act(async()=>cm.dispatch({changes:{from:3,insert:" draft"}}));expect(change).toHaveBeenLastCalledWith("new draft");
  await act(async()=>view.root.render(<TextDiffEditor content="new" baseline="old" syncVersion={1} onReview={review}/>));
  expect(cm.state.doc.toString()).toBe("new");
  await act(async()=>view.host.querySelector<HTMLButtonElement>('[aria-label="Undo all"]')!.click());
  expect(review).toHaveBeenLastCalledWith({content:"old",baseline:"old"});
  await act(async()=>view.root.render(<TextDiffEditor content="new" baseline="old" syncVersion={2} onReview={review}/>));
  expect(cm.state.doc.toString()).toBe("new");
 }finally{await view.close();}
});
