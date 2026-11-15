// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { undo, redo, undoDepth, redoDepth } from "@codemirror/commands";
import { EditorView } from "@codemirror/view";
import { afterEach, expect, it, vi } from "vitest";
import { createStructuredDocumentAdapter, HbsSession, projectDocumentReview, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { HbsStructuredEditor } from "./HbsStructuredEditor";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { structuredFixture } from "../../../app/previews/hbs-adapters/fixtures";

const cleanups:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const cleanup of cleanups.splice(0))await cleanup();vi.unstubAllGlobals();});
const initial={...structuredFixture,fields:[structuredFixture.fields[2]!,structuredFixture.fields[0]!,structuredFixture.fields[1]!]};
async function setup(content:unknown=initial){
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const adapter=createStructuredDocumentAdapter(directTextDiff);
  let saved:DocumentSnapshot<unknown>={version:1,saved:content,baseline:content};
  const session=await HbsSession.open({adapter,requestId:n=>`projection-${n}`,preservation:{load:()=>undefined,save:()=>{}},
    port:{read:async()=>saved,commit:async request=>{saved={version:saved.version+1,saved:request.saved,baseline:request.baseline};return saved;}}});
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  cleanups.push(async()=>{await act(async()=>root.unmount());session.dispose();host.remove();});
  await act(async()=>root.render(<HbsStructuredEditor session={session}/>));
  const native=(selector:string)=>{const surface=host.querySelector<HTMLElement>(selector);if(!surface)throw new Error(`missing ${selector}`);
    const editor=EditorView.findFromDOM(surface);if(!editor)throw new Error("missing CM editor");return editor;};
  const click=async(text:string)=>{const button=[...host.querySelectorAll<HTMLButtonElement>("[data-review-location-side] button"),...host.querySelectorAll<HTMLButtonElement>(".writing-diff-chunk-actions button,.hbs-structural-change button")].find(item=>item.textContent===text);
    if(!button)throw new Error(`missing ${text}`);await act(async()=>button.click());};
  const external=async(content:unknown)=>{saved={...saved,version:saved.version+1,saved:content};await act(async()=>session.syncLatest());};
  return {session,adapter,host,root,native,click,external,get saved(){return saved;}};
}

it("projects one field's old/new location and retains the native editor through Keep and history",async()=>{
  const f=await setup();
  const editor=f.native('[hidden] .cm-editor');
  const moved={...initial,fields:[initial.fields[0]!,initial.fields[2]!,initial.fields[1]!]};
  await f.external(moved);
  expect(f.session.getSnapshot().changes.map(change=>change.kind)).toEqual(["order"]);
  const old=f.host.querySelector('[data-review-location="before"]');
  if(!old)throw new Error("old location missing");
  expect(old?.querySelector('[data-field-id="approach"]')).not.toBeNull();
  expect(old?.textContent).toContain("比较不同噪声条件下的准确率。");
  expect(old?.querySelector(".cm-editor")).toBeNull();
  expect(f.host.querySelectorAll('[data-review-location="before"]')).toHaveLength(1);
  expect(f.host.querySelectorAll('[data-review-location="current"]')).toHaveLength(1);
  expect(f.host.querySelector('[data-field-id="key_result"]')?.closest(".editor-diff-added-block")).toBeNull();
  expect(f.host.querySelectorAll(".hbs-structural-change .editor-diff-added-block,.hbs-structural-change .editor-diff-removed-block")).toHaveLength(0);
  expect(f.native('[hidden] .cm-editor')).toBe(editor);
  expect(f.host.querySelectorAll('.hbs-structural-change')).toHaveLength(0);
  await f.click("Keep");
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.saved.baseline).toEqual(moved);
  expect(f.native('[hidden] .cm-editor')).toBe(editor);
  const history=f.native('[hidden] .cm-editor');
  await act(async()=>{expect(undo(history)).toBe(true);});
  expect(f.session.getSnapshot().changes.map(change=>change.kind)).toEqual(["order"]);
  expect(f.host.querySelectorAll('[data-review-location="before"]')).toHaveLength(1);
  await act(async()=>{expect(redo(history)).toBe(true);});
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});

it("Undo of a collection decision preserves its independently pending member text",async()=>{
  const f=await setup();
  const path=["fields",{key:"field",value:"approach"},"statements",{key:"id",value:"hbs-statement-1"},"text"] as const;
  const changed=f.adapter.replace({...initial,fields:[initial.fields[0]!,initial.fields[2]!,initial.fields[1]!] },path,"新的方法说明。");
  await f.external(changed);
  expect(f.session.getSnapshot().changes.map(change=>change.kind).sort()).toEqual(["order","text"]);
  await f.click("Undo");
  expect(f.adapter.read(f.session.getSnapshot().content,path)).toBe("新的方法说明。");
  expect(f.session.getSnapshot().changes.map(change=>change.kind)).toEqual(["text"]);
  expect(f.adapter.read(f.session.getSnapshot().baselineProjection,path)).toBe("比较不同噪声条件下的准确率。");
});

it("injects projection independently from review identity and native editors",async()=>{
  const f=await setup(), projector=vi.fn(projectDocumentReview);
  const editor=f.native('[hidden] .cm-editor');
  await act(async()=>f.root.render(<HbsStructuredEditor session={f.session} reviewProjector={projector}/>));
  await f.external({...initial,fields:[initial.fields[0]!,initial.fields[2]!,initial.fields[1]!]});
  expect(projector).toHaveBeenCalled();
  expect(f.host.querySelector('[data-review-location="before"]')?.getAttribute("data-hbs-review-decision")).toBe(f.session.getSnapshot().changes[0]!.id);
  expect(f.native('[hidden] .cm-editor')).toBe(editor);
});

it("uses the same member projection for a nested statement sequence inside its original card",async()=>{
  const statement=structuredFixture.fields[1]!.statements[0]!;
  const statements=["first","middle","last"].map((id,index)=>({...statement,id,text:`方法第 ${index+1} 项。`}));
  const content={...initial,fields:initial.fields.map(field=>field.field==="approach"?{...field,statements}:field)};
  const f=await setup(content);
  const cards=f.host.querySelectorAll('[data-field-id="approach"]');
  expect(cards).toHaveLength(1);
  expect(cards[0]!.querySelector(".cm-editor,textarea")).toBeNull();
  const moved={...content,fields:content.fields.map(field=>field.field==="approach"
    ?{...field,statements:[statements[0]!,statements[2]!,statements[1]!]}:field)};
  await f.external(moved);
  const old=f.host.querySelector('[data-review-location="before"]');
  expect(old?.querySelector("p")?.textContent).toBe("方法第 3 项。");
  expect(old?.closest('[data-field-id="approach"]')).not.toBeNull();
  expect(f.host.querySelectorAll('[data-field-id="approach"]')).toHaveLength(1);
  const current=f.host.querySelector('[data-field-id="approach"] .editor-diff-added-block');
  expect(current?.textContent).toContain("方法第 3 项。");
  expect(current?.querySelector('.cm-changedText,.cm-changedLine')).toBeNull();
  expect(current?.querySelector('.cm-editor')).toBeNull();
  expect(f.session.getSnapshot().changes.map(change=>change.kind)).toEqual(["order"]);
  expect(f.host.querySelectorAll(".hbs-structural-change")).toHaveLength(0);
  await f.click("Undo");
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.session.getSnapshot().content).toEqual(content);
});

it("keeps a deleted field's old slot before its surviving sibling and never adds it to H",async()=>{
  const f=await setup();
  await f.external({...initial,fields:[initial.fields[0]!,initial.fields[2]!]});
  const old=f.host.querySelector('[data-review-location="before"]');
  expect(old?.querySelector('[data-field-id="research_question"]')).not.toBeNull();
  expect(old?.textContent).toContain("局部观察如何支持逻辑推理？");
  const current=f.host.querySelector('[data-field-id="approach"]');
  expect(old!.compareDocumentPosition(current!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(f.session.getSnapshot().content).toEqual({...initial,fields:[initial.fields[0]!,initial.fields[2]!]});
  expect(old?.querySelector('.cm-editor')).toBeNull();
  await f.click("Keep");
  expect(f.host.querySelector('[data-field-id="research_question"]')).toBeNull();
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});

it("jumps between paired locations in its own instance without changing data or native history",async()=>{
  const f=await setup(), other=await setup();
  const moved={...initial,fields:[initial.fields[0]!,initial.fields[2]!,initial.fields[1]!]};
  await f.external(moved);await other.external(moved);
  const before=f.host.querySelector<HTMLElement>('[data-review-location-target="before"]');
  const after=f.host.querySelector<HTMLElement>('[data-review-location-target="after"]');
  if(!before||!after)throw new Error("paired locations missing");
  const beforeScroll=vi.fn(),afterScroll=vi.fn();
  before.scrollIntoView=beforeScroll;after.scrollIntoView=afterScroll;
  const old=f.host.querySelector<HTMLButtonElement>('[data-review-location-side="after"] [data-review-location-jump]');
  const next=f.host.querySelector<HTMLButtonElement>('[data-review-location-side="before"] [data-review-location-jump]');
  if(!old||!next)throw new Error("navigation missing");
  expect(old.textContent).toBe("old");expect(next.textContent).toBe("new");
  const snapshot=f.session.getSnapshot(),otherSnapshot=other.session.getSnapshot();
  const editor=f.native('[hidden] .cm-editor');
  const history=[undoDepth(editor.state),redoDepth(editor.state)];
  await act(async()=>old.click());
  expect(beforeScroll).toHaveBeenCalledOnce();expect(document.activeElement).toBe(next);
  await act(async()=>next.click());
  expect(afterScroll).toHaveBeenCalledOnce();expect(document.activeElement).toBe(old);
  expect(f.session.getSnapshot()).toBe(snapshot);expect(other.session.getSnapshot()).toBe(otherSnapshot);
  expect([undoDepth(editor.state),redoDepth(editor.state)]).toEqual(history);
  expect(f.host.querySelectorAll(".hbs-structural-change button")).toHaveLength(0);
  const beforeButtons=[...before.querySelectorAll("button")].map(button=>button.textContent);
  const afterButtons=[...after.querySelectorAll("button")].map(button=>button.textContent);
  expect(beforeButtons).toEqual(["new","Undo"]);expect(afterButtons).toEqual(["old","Keep"]);
});

it.each(["Keep","Undo"])("keeps a stable native history owner after deleting every field and resolving %s",async action=>{
  const single={...structuredFixture,fields:[structuredFixture.fields[0]!]};
  const empty={...single,fields:[]};
  const f=await setup(single),history=f.native("[hidden] .cm-editor");
  await f.external(empty);
  expect(f.host.querySelectorAll('.structured-resource-cards .cm-editor')).toHaveLength(0);
  expect(history.contentDOM.contentEditable).toBe("false");
  expect(history.contentDOM.getAttribute("aria-readonly")).toBe("true");
  await f.click(action);
  const accepted=f.session.getSnapshot().content;
  expect(accepted).toEqual(action === "Keep" ? empty : single);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.native("[hidden] .cm-editor")).toBe(history);
  await act(async()=>{expect(undo(history)).toBe(true);});
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.session.getSnapshot().content).toEqual(empty);
  await act(async()=>{expect(redo(history)).toBe(true);});
  expect(f.session.getSnapshot().content).toEqual(accepted);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.session.getSnapshot().events.join(" ")).not.toContain("MODEL_HISTORY_UNAVAILABLE");
});

it("reviews resources whose real cards never had editable leaves without allowing format input",async()=>{
  const content={...structuredFixture,fields:structuredFixture.fields.slice(2)};
  const f=await setup(content),history=f.native("[hidden] .cm-editor");
  expect(f.host.querySelectorAll('.structured-resource-cards .cm-editor')).toHaveLength(0);
  const format=history.state.doc.toString();
  await act(async()=>history.dispatch({changes:{from:0,insert:"unauthorized"},userEvent:"input.type"}));
  expect(history.state.doc.toString()).toBe(format);
  expect(f.session.getSnapshot().content).toEqual(content);
  const moved={...content,fields:[content.fields[3]!,...content.fields.slice(0,3)]};
  await f.external(moved);
  await f.click("Keep");expect(f.session.getSnapshot().changes).toHaveLength(0);
  await act(async()=>{expect(undo(history)).toBe(true);});
  expect(f.session.getSnapshot().changes.map(change=>change.kind)).toEqual(["order"]);
  expect(history.state.doc.toString()).toBe(format);
  await f.click("Undo");expect(f.session.getSnapshot().content).toEqual(content);
});

it("anchors deletions and moves across independent decisions to the same stable successor",async()=>{
  const base=structuredFixture.fields[0]!,statement=base.statements[0]!;
  const fields=["a","b","c","d"].map(field=>({...base,field,statements:[{...statement,id:`${field}-statement`,text:field.toUpperCase()}]}));
  const content={...structuredFixture,fields};
  const f=await setup(content);
  await f.external({...content,fields:[fields[2]!,fields[0]!,fields[3]!]});
  const old=[...f.host.querySelectorAll('[data-review-location="before"]')];
  expect(old.map(element=>element.querySelector('p')?.textContent)).toEqual(["B","C"]);
  const stableA=f.host.querySelector('[data-field-id="a"]')!,stableD=f.host.querySelector('[data-field-id="d"]')!;
  for(const fragment of old){
    expect(stableA.compareDocumentPosition(fragment)&Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(fragment.compareDocumentPosition(stableD)&Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  }
});

async function editField(host:HTMLElement,label:string,text:string){
  const edit=host.querySelector<HTMLButtonElement>(`button[aria-label="编辑${label}"]`)!;
  await act(async()=>edit.click());
  const input=host.querySelector<HTMLTextAreaElement>(`textarea[aria-label="编辑${label}正文"]`)!;
  await act(async()=>{
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,text);
    input.dispatchEvent(new Event("input",{bubbles:true}));
  });
}
it("preserves the production Pencil form's live candidate in S′ without accepting or saving S",async()=>{
  const f=await setup();
  const path=["fields",{key:"field",value:"approach"},"statements",{key:"id",value:"hbs-statement-1"},"text"] as const;
  await f.external(f.adapter.replace(f.saved.saved,path,"incoming 85%"));
  await editField(f.host,"approach","my candidate 91%");
  expect(f.adapter.read(f.session.getSnapshot().content,path)).toBe("my candidate 91%");
  expect(f.adapter.read(f.session.getSnapshot().preserved,path)).toBe("my candidate 91%");
  expect(f.adapter.read(f.saved.saved,path)).toBe("incoming 85%");
  expect(f.adapter.read(f.saved.baseline,path)).toBe("比较不同噪声条件下的准确率。");
  expect(f.host.querySelectorAll('[data-field-id="approach"] .cm-editor')).toHaveLength(0);
  expect(f.host.querySelector('[data-field-id="approach"] [aria-label="修改前"]')?.textContent).toBe("比较不同噪声条件下的准确率。");
  expect(f.host.querySelector('[data-field-id="approach"] [aria-label="修改后"] textarea')).not.toBeNull();
  await act(async()=>f.host.querySelector<HTMLTextAreaElement>("textarea")!.blur());
  expect(f.host.querySelector("textarea")).toBeNull();
  expect(f.adapter.read(f.saved.saved,path)).toBe("my candidate 91%");
  await f.external(f.adapter.replace(f.saved.saved,path,"incoming 90%"));
  expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
  expect(f.host.querySelector('[aria-label="incoming"]')?.textContent).toBe("incoming 90%");
  const old=f.host.querySelector('[aria-label="修改前"]')!,incoming=f.host.querySelector('[aria-label="incoming"]')!,current=f.host.querySelector('[aria-label="修改后"]')!;
  expect(old.compareDocumentPosition(incoming)&Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(incoming.compareDocumentPosition(current)&Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

it("double-clicks only the new side into editing while preserving old, incoming and review controls",async()=>{
  const f=await setup();
  const path=["fields",{key:"field",value:"approach"},"statements",{key:"id",value:"hbs-statement-1"},"text"] as const;
  await f.external(f.adapter.replace(f.saved.saved,path,"新的方法说明。"));
  const card=f.host.querySelector('[data-field-id="approach"]')!;
  await act(async()=>card.querySelector('[aria-label="修改前"]')!.dispatchEvent(new MouseEvent("dblclick",{bubbles:true})));
  expect(card.querySelector("textarea")).toBeNull();
  await act(async()=>card.querySelector('[aria-label="修改后"]')!.dispatchEvent(new MouseEvent("dblclick",{bubbles:true})));
  expect(card.querySelector('[aria-label="修改后"] textarea')).not.toBeNull();
  expect(card.querySelector('[aria-label="修改前"]')?.textContent).toBe("比较不同噪声条件下的准确率。");
  expect(card.querySelector('[aria-label="保留第 1 处变更"]')).not.toBeNull();
  expect([...card.querySelectorAll("button")].map(button=>button.textContent)).not.toContain("保存");
  await act(async()=>card.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
  expect(card.querySelector("textarea")).toBeNull();
  expect(f.adapter.read(f.saved.saved,path)).toBe("新的方法说明。");
  expect(f.session.getSnapshot().changes).toHaveLength(1);
});

it("Escape saves textarea input while retaining compatible external field changes",async()=>{
  const f=await setup();
  const path=["fields",{key:"field",value:"approach"},"statements",{key:"id",value:"hbs-statement-1"},"text"] as const;
  const other=["fields",{key:"field",value:"research_question"},"statements",{key:"id",value:"hbs-statement-0"},"text"] as const;
  await editField(f.host,"approach","ordinary typing");
  await f.external(f.adapter.replace(f.saved.saved,other,"external question"));
  await act(async()=>f.host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
  expect(f.adapter.read(f.session.getSnapshot().content,path)).toBe("ordinary typing");
  expect(f.adapter.read(f.saved.saved,path)).toBe("ordinary typing");
  expect(f.adapter.read(f.session.getSnapshot().content,other)).toBe("external question");
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.adapter.read(f.session.getSnapshot().preserved,path)).toBe("ordinary typing");
  expect(f.host.querySelector("textarea")).toBeNull();
});

it.each(["keep","undo"] as const)("reviews live input with one pointer click on %s, without a preceding blur save",async action=>{
  const f=await setup();
  const path=["fields",{key:"field",value:"approach"},"statements",{key:"id",value:"hbs-statement-1"},"text"] as const;
  const original=String(f.adapter.read(f.saved.baseline,path));
  await f.external(f.adapter.replace(f.saved.saved,path,"incoming 85%"));
  await editField(f.host,"approach","my candidate 91%");
  const input=f.host.querySelector("textarea")!;
  const button=f.host.querySelector<HTMLButtonElement>(`[data-field-id="approach"] .writing-diff-chunk-${action}`)!;
  expect(button.disabled).toBe(false);expect(document.activeElement).toBe(input);
  const down=new PointerEvent("pointerdown",{button:0,bubbles:true,cancelable:true});
  await act(async()=>button.dispatchEvent(down));
  expect(down.defaultPrevented).toBe(true);
  expect(f.adapter.read(f.saved.saved,path)).toBe("incoming 85%");
  await act(async()=>{button.dispatchEvent(new PointerEvent("pointerup",{button:0,bubbles:true}));button.click();});
  const expected=action==="keep"?"my candidate 91%":original;
  await vi.waitFor(()=>expect(f.adapter.read(f.saved.saved,path)).toBe(expected));
  expect(f.adapter.read(f.saved.baseline,path)).toBe(expected);
  expect(f.host.querySelector("textarea")).toBeNull();
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});

it("allows keyboard review focus without submitting, and commits the latest input on Keep",async()=>{
  const f=await setup();
  const path=["fields",{key:"field",value:"approach"},"statements",{key:"id",value:"hbs-statement-1"},"text"] as const;
  await f.external(f.adapter.replace(f.saved.saved,path,"incoming 85%"));
  await editField(f.host,"approach","my candidate 91%");
  const button=f.host.querySelector<HTMLButtonElement>('[aria-label="应用当前变更"]')!;
  expect(button.disabled).toBe(false);
  await act(async()=>button.focus());
  expect(f.host.querySelector("textarea")).not.toBeNull();
  expect(f.adapter.read(f.saved.saved,path)).toBe("incoming 85%");
  await act(async()=>button.click());
  await vi.waitFor(()=>expect(f.adapter.read(f.saved.baseline,path)).toBe("my candidate 91%"));
  expect(f.host.querySelector("textarea")).toBeNull();
});

it("auto-saves without accepting when keyboard focus leaves review controls",async()=>{
  const f=await setup();
  const path=["fields",{key:"field",value:"approach"},"statements",{key:"id",value:"hbs-statement-1"},"text"] as const;
  const original=f.adapter.read(f.saved.baseline,path);
  await f.external(f.adapter.replace(f.saved.saved,path,"incoming 85%"));
  await editField(f.host,"approach","my candidate 91%");
  await act(async()=>f.host.querySelector<HTMLButtonElement>('[aria-label="应用当前变更"]')!.focus());
  const outside=document.createElement("button");document.body.append(outside);
  try{await act(async()=>outside.focus());}finally{outside.remove();}
  await vi.waitFor(()=>expect(f.adapter.read(f.saved.saved,path)).toBe("my candidate 91%"));
  expect(f.adapter.read(f.saved.baseline,path)).toBe(original);
  expect(f.session.getSnapshot().changes).toHaveLength(1);
});

it("keeps bulk review available through its portaled menu while editing",async()=>{
  const f=await setup();
  const path=["fields",{key:"field",value:"approach"},"statements",{key:"id",value:"hbs-statement-1"},"text"] as const;
  await f.external(f.adapter.replace(f.saved.saved,path,"incoming 85%"));
  await editField(f.host,"approach","my candidate 91%");
  const more=f.host.querySelector<HTMLButtonElement>('[aria-label="更多阅读审阅操作"]')!;
  expect(more.disabled).toBe(false);
  await act(async()=>more.click());
  const keep=[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(button=>button.textContent==="Keep all")!;
  expect(keep).toBeDefined();
  await act(async()=>keep.focus());
  expect(f.adapter.read(f.saved.saved,path)).toBe("incoming 85%");
  await act(async()=>keep.click());
  await vi.waitFor(()=>expect(f.adapter.read(f.saved.baseline,path)).toBe("my candidate 91%"));
  expect(f.host.querySelector("textarea")).toBeNull();
});

it("retains the host's actual busy protection while a field is being edited",async()=>{
  const f=await setup();
  const path=["fields",{key:"field",value:"approach"},"statements",{key:"id",value:"hbs-statement-1"},"text"] as const;
  await f.external(f.adapter.replace(f.saved.saved,path,"incoming 85%"));
  await editField(f.host,"approach","my candidate 91%");
  await act(async()=>f.root.render(<HbsStructuredEditor session={f.session} busy/>));
  expect(f.host.querySelector<HTMLButtonElement>('[aria-label="应用当前变更"]')!.disabled).toBe(true);
  expect(f.host.querySelector<HTMLButtonElement>('[aria-label="保留第 1 处变更"]')!.disabled).toBe(true);
  expect(f.host.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("my candidate 91%");
  expect(f.adapter.read(f.saved.saved,path)).toBe("incoming 85%");
});

it("retains native review history when the same resource hands off to another production host",async()=>{
  const f=await setup();
  const moved={...initial,fields:[initial.fields[0]!,initial.fields[2]!,initial.fields[1]!]};
  await f.external(moved);await f.click("Keep");
  const previous=f.native('[hidden] .cm-editor');
  await act(async()=>f.root.render(null));
  await act(async()=>f.root.render(<HbsStructuredEditor session={f.session}/>));
  const editor=f.native('[hidden] .cm-editor');
  expect(editor).not.toBe(previous);
  await act(async()=>{expect(undo(editor)).toBe(true);});
  expect(f.session.getSnapshot().changes.map(change=>change.kind)).toEqual(["order"]);
  await act(async()=>{expect(redo(editor)).toBe(true);});
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});
