/** @vitest-environment happy-dom */
import {afterEach, expect, it, vi} from "vitest";
import {copyText} from "./copyText";
afterEach(() => {document.body.replaceChildren(); vi.restoreAllMocks();});

it("preserves rich formats, focus and text selection through the webview fallback", async () => {
  const button=document.createElement("button"), paragraph=document.createElement("p");
  paragraph.textContent="selected text"; document.body.append(button,paragraph); button.focus();
  const range=document.createRange();range.selectNodeContents(paragraph);document.getSelection()!.addRange(range);
  Object.defineProperty(navigator,"clipboard",{configurable:true,value:{writeText:vi.fn()}});
  const values=new Map<string,string>();
  Object.defineProperty(document,"execCommand",{configurable:true,value:vi.fn(()=>{
    const event=new Event("copy",{cancelable:true,bubbles:true});
    Object.defineProperty(event,"clipboardData",{value:{setData:(type:string,value:string)=>values.set(type,value)}});
    document.dispatchEvent(event); return true;
  })});
  expect(await copyText("@tag",{"text/html":"<span>tag metadata</span>"})).toBe(true);
  expect(values.get("text/html")).toBe("<span>tag metadata</span>");
  expect(values.get("text/plain")).toBe("@tag");
  expect(navigator.clipboard.writeText).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(button);
  expect(document.getSelection()!.toString()).toBe("selected text");
});

it("does not report rich copy success if the clipboard event cannot preserve metadata", async () => {
  Object.defineProperty(navigator,"clipboard",{configurable:true,value:{writeText:vi.fn()}});
  Object.defineProperty(document,"execCommand",{configurable:true,value:vi.fn(()=>true)});
  expect(await copyText("@tag",{"text/html":"metadata"})).toBe(false);
  expect(navigator.clipboard.writeText).not.toHaveBeenCalled();
});
