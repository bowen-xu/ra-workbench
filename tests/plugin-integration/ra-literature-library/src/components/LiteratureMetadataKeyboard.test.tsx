// @vitest-environment happy-dom
import { act, StrictMode, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { emptyLiteratureDraft, type LiteratureItem, type LiteratureMetadataPatch } from "@research-assistant/core/literature";
import { LiteratureMetadataDialog } from "../../../../../../plugins/ra-literature-library/src/components/LiteratureMetadataDialog";
import { ApplicationKeyboardPolicy } from "../../../../../apps/web/src/core/runtime/ApplicationKeyboardPolicy";

const initial: LiteratureItem = {...emptyLiteratureDraft(), id:"paper", itemType:"journalArticle", title:"Keyboard paper", issuedYear:2026,
  containerTitle:"Journal", abstractText:"Abstract", attachments:[], projectIds:[], readingStatus:"unread", metadataSources:[],
  creators:[{familyName:"Lee",givenName:"Bo",literalName:"",role:"author"}], createdAt:"",updatedAt:""};
let root: Root, host: HTMLDivElement;
const save = vi.fn<(patch: LiteratureMetadataPatch) => Promise<LiteratureItem>>();
const onError = vi.fn();

function Harness({readOnly = false}: {readonly readOnly?: boolean}) {
  const [item,setItem] = useState(initial);
  return <><ApplicationKeyboardPolicy /><LiteratureMetadataDialog item={item} isReadOnly={readOnly} focusFieldName={undefined} onClose={() => {}}
    onError={onError} onSuccess={() => {}} onUpdateField={async patch => {const next=await save(patch);setItem(next);return next;}}
    onConfirmFields={async () => item} onPreviewDoi={vi.fn()} onApplyCandidateFields={async () => item} onRejectCandidate={async () => {}} /></>;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  save.mockReset().mockImplementation(async patch => ({...initial,[patch.fieldName]:patch.value}) as LiteratureItem);
  onError.mockReset();
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async () => {await act(async () => root.unmount());host.remove();vi.unstubAllGlobals();});

async function mount(readOnly = false) {await act(async () => root.render(<StrictMode><Harness readOnly={readOnly}/></StrictMode>));}
async function edit(field: string) {
  await act(async () => document.querySelector(`[data-metadata-field="${field}"]`)?.dispatchEvent(new MouseEvent("dblclick",{bubbles:true})));
}
function input(label: string): HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement {
  const element=document.querySelector(`input[aria-label="${label}"],textarea[aria-label="${label}"],select[aria-label="${label}"]`);
  if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement || element instanceof HTMLSelectElement)) throw new Error(`Missing editor: ${label}`);
  return element;
}
async function fill(label: string,value: string) {
  const element=input(label);
  const type=element instanceof HTMLSelectElement ? HTMLSelectElement : element instanceof HTMLTextAreaElement ? HTMLTextAreaElement : HTMLInputElement;
  await act(async () => {Object.getOwnPropertyDescriptor(type.prototype,"value")?.set?.call(element,value);element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input",{bubbles:true}));});
}
async function tab(target: Element,shiftKey = false) {
  const event=new KeyboardEvent("keydown",{key:"Tab",shiftKey,bubbles:true,cancelable:true});
  await act(async () => {target.dispatchEvent(event);});
  return event;
}

it("saves once and enters the next/previous input in visible field order", async () => {
  await mount();await edit("issuedYear");await fill("年份","2030");
  expect((await tab(input("年份"))).defaultPrevented).toBe(true);
  expect(save).toHaveBeenCalledExactlyOnceWith({fieldName:"issuedYear",value:2030});
  expect(document.activeElement).toBe(input("期刊"));
  await tab(input("期刊"),true);
  expect(document.activeElement).toBe(input("年份"));
  expect(input("年份").value).toBe("2030");
  expect(save).toHaveBeenCalledOnce();
});

it("skips actions and fields unavailable for the selected literature type", async () => {
  await mount();await edit("doi");await tab(input("DOI"));
  expect(document.activeElement).toBe(input("URL"));
  await edit("itemType");await fill("条目类型","webpage");await tab(input("条目类型"));
  expect(document.activeElement).toBe(input("年份"));
  await tab(input("年份"));
  expect(document.activeElement?.getAttribute("aria-label")).not.toBe("会议名称");
  expect(document.querySelector('[data-metadata-field="conferenceName"]')).toBeNull();
});

it("keeps the draft and focus when validation or persistence fails", async () => {
  await mount();await edit("title");await fill("标题","");await tab(input("标题"));
  expect(document.querySelector('[role="alert"]')?.textContent).toBe("标题不能为空");
  expect(document.activeElement).toBe(input("标题"));expect(save).not.toHaveBeenCalled();
  await fill("标题","Draft title");save.mockRejectedValueOnce(new Error("storage_error: write failed"));
  await tab(input("标题"));
  expect(input("标题").value).toBe("Draft title");
  expect(document.querySelector('.metadata-creator-editor')).toBeNull();
  await vi.waitFor(() => expect(document.activeElement).toBe(input("标题")));
  expect(onError).toHaveBeenCalledWith("storage_error: write failed");
});

it("does not submit a second write from Tab or blur while a save is pending", async () => {
  let resolve: ((value: LiteratureItem) => void) | undefined;
  save.mockImplementationOnce(() => new Promise(done => {resolve=done;}));
  await mount();await edit("issuedYear");await fill("年份","2031");
  const year=input("年份");await tab(year);
  await tab(year);await act(async () => {year.dispatchEvent(new FocusEvent("focusout",{bubbles:true}));});
  expect(save).toHaveBeenCalledOnce();expect(year.disabled).toBe(true);
  await act(async () => resolve?.({...initial,issuedYear:2031}));
  expect(document.activeElement).toBe(input("期刊"));
});

it("moves between creator controls and crosses their field boundaries under the application policy", async () => {
  await mount();await edit("creators");
  expect((await tab(input("创作者姓名 1"))).defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(input("创作者角色 1"));
  await tab(input("创作者姓名 1"),true);
  expect(document.activeElement).toBe(input("标题"));
  await tab(input("标题"));
  expect(document.activeElement).toBe(input("创作者姓名 1"));
  await tab(input("创作者角色 1"));
  expect(document.activeElement).toBe(input("条目类型"));
  await tab(input("条目类型"),true);
  expect(document.activeElement).toBe(input("创作者角色 1"));
  expect(save).not.toHaveBeenCalled();
});

it("loops between boundary inputs without focusing action buttons", async () => {
  await mount();await edit("abstractText");await tab(input("摘要"));
  expect(document.activeElement).toBe(input("标题"));
  await tab(input("标题"),true);
  expect(document.activeElement).toBe(input("摘要"));
  expect(save).not.toHaveBeenCalled();
});

it("provides an unsaved input when entering an empty creator field", async () => {
  await mount();await edit("creators");
  await act(async () => (document.querySelector('button[aria-label="移除创作者 1"]') as HTMLButtonElement).click());
  await act(async () => (document.querySelector('button[aria-label="保存创作者"]') as HTMLButtonElement).click());
  await edit("title");await tab(input("标题"));
  expect(document.activeElement).toBe(input("创作者姓名 1"));
  expect(input("创作者姓名 1").value).toBe("");
  await tab(document.activeElement!,true);
  expect(document.activeElement).toBe(input("标题"));
});

it("keeps trash metadata read-only", async () => {
  await mount(true);await edit("title");
  expect(document.querySelector('.metadata-field input')).toBeNull();
  expect(save).not.toHaveBeenCalled();
});
