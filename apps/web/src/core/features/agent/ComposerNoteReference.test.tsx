// @vitest-environment happy-dom
import {act, type ReactElement, type ReactNode} from "react";
import {EditorState} from "@codemirror/state";
import {EditorView} from "@codemirror/view";
import {deleteCharBackward, undo} from "@codemirror/commands";
import {afterEach, expect, it, vi} from "vitest";
import type {AgentReferenceInput} from "@research-assistant/core/agent";
import {composerBindingsEffect, composerEditorDraft, composerReferenceExtensions} from "./agentComposerEditor";
import {composerNoteId} from "./ComposerNoteReference";
import {panelItem} from "../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";
import {sourceLinkHint} from "./sourceLinkHint";

const ports=vi.hoisted(()=>({loadDocument:vi.fn(),listOutgoing:vi.fn(),get:vi.fn()}));
vi.mock("../notes/gateways",()=>({getNotesGateway:()=>ports}));
vi.mock("../../services/sourceDomainGateways",()=>({getLiteratureGateway:()=>ports}));
vi.mock("../../components/ControlHint",()=>({ControlHint:({children,content}:{children:ReactElement;content:ReactNode})=><>{children}<aside>{content}</aside></>}));
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
let view:EditorView|undefined;
afterEach(async()=>{await act(async()=>{view?.destroy();view=undefined;});document.body.replaceChildren();vi.clearAllMocks();});
const reference={type:"mention",name:"Paper title · 旧笔记标题",target:{kind:"file",sourceId:'context:{"kind":"note","resourceId":"note"}',resourceId:"note",revision:"current"}} as const satisfies AgentReferenceInput;

it("shows the note title first for restored references and a full formatted paper in the shared hint",async()=>{
  const item=panelItem(), open=vi.fn();
  ports.loadDocument.mockResolvedValue({page:{title:"算法整理：完整笔记标题"}});
  ports.listOutgoing.mockResolvedValue([{relation:"derived_from",target:{kind:"paper",id:item.id},isBroken:false},
    {relation:"reference",target:{kind:"paper",id:"unrelated"},isBroken:false}]);
  ports.get.mockResolvedValue(item);
  const host=document.createElement("div");document.body.append(host);
  await act(async()=>{
    view=new EditorView({parent:host,state:EditorState.create({doc:"@旧标签",extensions:[composerReferenceExtensions(open)]})});
    view.dispatch({effects:composerBindingsEffect.of([{startUtf16:0,endUtf16:4,input:reference}])});
  });
  expect(host.querySelector(".agent-composer-reference-label")?.textContent).toBe("算法整理：完整笔记标题");
  expect([...host.querySelectorAll("aside span.control-hint-lines > span")].map(line=>line.textContent)).toEqual([
    "算法整理：完整笔记标题",sourceLinkHint({kind:"paper",item})[0],
  ]);
  expect(ports.get).toHaveBeenCalledExactlyOnceWith(item.id);
  await act(async()=>host.querySelector<HTMLAnchorElement>("a")!.click());
  expect(open).toHaveBeenCalledWith(reference);
  await act(async()=>{view!.dispatch({selection:{anchor:4}});expect(deleteCharBackward(view!)).toBe(true);});
  expect(composerEditorDraft(view!.state).bindings).toHaveLength(0);
  await act(async()=>{expect(undo(view!)).toBe(true);});
  expect(composerEditorDraft(view!.state).bindings[0]?.input).toEqual(reference);
});

it("keeps an unavailable note's draft and exact binding without inventing a parent paper",async()=>{
  ports.loadDocument.mockRejectedValue(new Error("missing"));
  const host=document.createElement("div");document.body.append(host);
  await act(async()=>{
    view=new EditorView({parent:host,state:EditorState.create({doc:"@旧标签",extensions:[composerReferenceExtensions(()=>undefined)]})});
    view.dispatch({effects:composerBindingsEffect.of([{startUtf16:0,endUtf16:4,input:reference}])});
  });
  expect(host.querySelector(".agent-composer-reference-label")?.textContent).toBe(reference.name);
  expect(composerEditorDraft(view!.state).bindings[0]?.input).toEqual(reference);
  expect(ports.get).not.toHaveBeenCalled();
});

it("only resolves note bindings with valid managed resource identities",()=>{
  expect(composerNoteId(reference)).toBe("note");
  expect(composerNoteId({...reference,target:{...reference.target,sourceId:"context:not-json"}})).toBeUndefined();
  expect(composerNoteId({...reference,target:{...reference.target,sourceId:'context:{"kind":"paper","resourceId":"paper"}'}})).toBeUndefined();
});
