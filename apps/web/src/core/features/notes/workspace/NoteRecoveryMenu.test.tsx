// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {NoteRecoveryMenu} from "./NoteRecoveryMenu";
import {NoteSaveState} from "./NoteSaveState";
const ports=vi.hoisted(()=>({collect:vi.fn(),save:vi.fn(),notes:{mock:true}}));
vi.mock("../gateways",()=>({getNotesGateway:()=>ports.notes}));
vi.mock("../transfer/noteRecoveryExport",()=>({collectNoteRecovery:ports.collect}));
vi.mock("../transfer/saveNoteRecoveryFile",()=>({saveNoteRecoveryFile:ports.save}));
let host:HTMLDivElement,root:ReturnType<typeof createRoot>;
beforeEach(()=>{vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);host=document.createElement("div");document.body.append(host);root=createRoot(host);ports.collect.mockReset();ports.save.mockReset();});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});
it.each(["recovery","diagnostics"] as const)("uses the shared export menu and collects %s without any resolution action",async kind=>{
  ports.collect.mockResolvedValue({missing:[],diagnostics:{kind:"log"}});ports.save.mockResolvedValue("saved");
  await act(async()=>root.render(<NoteRecoveryMenu pageId="mock-page" label="保存失败：MODEL_TEST" status="failed" message="MODEL_TEST"/>));
  const trigger=host.querySelector<HTMLButtonElement>("button")!;
  await act(async()=>trigger.click());
  const menu=document.querySelector('[role="menu"][aria-label="笔记恢复与诊断"]')!;
  expect(menu.closest('[data-overlay-portal="popup"]')).not.toBeNull();
  expect(menu.textContent).not.toContain("另存副本");
  const label=kind==="recovery"?"导出数据恢复包":"导出错误日志";
  await act(async()=>[...menu.querySelectorAll<HTMLButtonElement>("button")].find(item=>item.textContent===label)!.click());
  expect(ports.collect).toHaveBeenCalledExactlyOnceWith("mock-page",ports.notes,{status:"failed",message:"MODEL_TEST"},kind==="recovery");
  expect(ports.save.mock.calls[0]![1]).toEqual(kind==="recovery"?{missing:[],diagnostics:{kind:"log"}}:{kind:"log"});
});
it("Escape closes only the menu and returns focus to its status icon",async()=>{
  await act(async()=>root.render(<NoteRecoveryMenu pageId="mock-page" label="版本冲突" status="conflict" message={undefined}/>));
  const trigger=host.querySelector<HTMLButtonElement>("button")!;
  await act(async()=>trigger.click());
  const menu=document.querySelector('[role="menu"]')!;
  await act(async()=>menu.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
  await vi.waitFor(()=>expect(document.querySelector('[role="menu"]')).toBeNull());
  expect(document.activeElement).toBe(trigger);expect(ports.collect).not.toHaveBeenCalled();
});
it("makes the existing conflict status icon open the same menu without inventing a content decision",async()=>{
  await act(async()=>root.render(<NoteSaveState pageId="mock-page" status="conflict" message="MODEL_UNRESOLVED"/>));
  expect(host.querySelectorAll("button")).toHaveLength(1);
  await act(async()=>host.querySelector<HTMLButtonElement>("button")!.click());
  const menu=document.querySelector('[role="menu"][aria-label="笔记恢复与诊断"]')!;
  expect(menu.textContent).toContain("导出错误日志");expect(menu.textContent).not.toContain("保留当前内容并解决冲突");
  expect(ports.collect).not.toHaveBeenCalled();
});
