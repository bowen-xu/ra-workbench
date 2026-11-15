/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {ToolActivityHistoryRow} from "./ToolActivityHistoryRow";
import {mergeToolReadRows} from "./toolReadActivity";
import {toolActivityRows} from "./toolActivityModel";
import {DeveloperModeProvider,DeveloperModeToggle} from "../../settings/DeveloperMode";
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
const patch="*** Begin Patch\n*** Update File: preview.md\n@@\n-old text\n+new text\n*** End Patch";
it("shows a colored edit only after an applied receipt, retaining rejected inputs and errors as plain text",async()=>{
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const call={type:"tool-call" as const,callId:"patch",name:"research_apply_patch",content:JSON.stringify({patch}),turnFinished:true};
 const result={type:"tool-result" as const,callId:"patch",name:"research_apply_patch",content:JSON.stringify({status:"applied",targets:[{path:"preview.md",status:"applied"}]})};
 async function render(receipt:typeof result|undefined,error=false) {
  const row=toolActivityRows([call,...receipt?[{...receipt,error}]:[]])[0]!;
  await act(async()=>root.render(<ToolActivityHistoryRow row={row}/>));
 }
 try {
  await render(undefined);
  expect(host.querySelector(".tool-patch-diff")).toBeNull();
  await render(result);
  expect(host.querySelector(".tool-row-compact")?.textContent).toContain("已编辑 preview.md");
  expect(host.querySelector(".tool-row-expanded")?.textContent).toContain("已编辑 preview.md");
  expect(host.querySelector(".added code")?.textContent).toContain("new text");
  expect(host.querySelector(".deleted code")?.textContent).toContain("old text");
  const details=host.querySelector("details")!;details.open=true;
  await render({...result,content:'{"error":"patch_conflict","message":"context not found"}'},true);
  expect(details.open).toBe(true);
  expect(host.querySelector(".tool-patch-diff")).toBeNull();
  expect(host.querySelector(".added, .deleted")).toBeNull();
  expect(host.querySelector(".tool-row-expanded")?.textContent).toBe("无法编辑 preview.md");
  expect(host.querySelector('[aria-label="调用参数"]')).toBeNull();
  expect(host.querySelector('[aria-label="调用结果"]')?.textContent).toContain("context not found");
  expect(host.querySelectorAll(".tool-content-card")).toHaveLength(1);
  expect(host.querySelector(".tool-shell-content")?.textContent).toContain("对象：preview.md");
  expect(host.querySelector(".tool-shell-content")?.textContent).toContain("错误码：patch_conflict");
  expect(host.querySelector(".tool-shell-content")?.textContent).not.toContain("*** Begin Patch");
  expect(host.querySelector(".tool-shell-hint")).toBeNull();
  expect(host.querySelector(".tool-shell-card > header")?.textContent).toBe("编辑结果");
  expect([...host.querySelectorAll("header")].map(header=>header.textContent)).not.toContain("文件修改");
  await render({...result,content:'{"status":"complete","targets":[{"path":"preview.md","status":"proposed"}]}'});
  expect(host.querySelector(".tool-patch-content")).toBeNull();
 }finally{await act(async()=>root.unmount());host.remove();}
});
it("keeps read-file protocol data behind the developer-only inline disclosure",async()=>{
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const row=toolActivityRows([{type:"tool-call",callId:"read",name:"research_read_file",content:'{"path":"paper.md"}'},
  {type:"tool-result",callId:"read",content:'{"content":"original file content","revision":7}'}])[0]!;
 try {
  await act(async()=>root.render(<DeveloperModeProvider><DeveloperModeToggle/><ToolActivityHistoryRow row={row}/></DeveloperModeProvider>));
  expect(host.textContent).toContain("paper.md");
  expect(host.textContent).not.toContain("original file content");
  expect(host.querySelector(".tool-history-row details, .tool-history-row pre, .tool-history-row button")).toBeNull();
  expect(host.querySelector(".tool-history-row")?.tagName).toBe("DIV");
  await act(async()=>host.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  expect(host.querySelector("details, .writing-agent-activity-chevron")).toBeNull();
  const button=host.querySelector<HTMLButtonElement>('[aria-label="查看原始数据"]')!;
  expect(button.closest(".tool-history-static-summary")).not.toBeNull();
  await act(async()=>button.click());
  expect(host.querySelector("details")).toBeNull();
  expect(host.querySelector('pre[aria-label="消息原始数据"]')?.textContent).toContain("original file content");
  expect(document.querySelector("dialog")).toBeNull();
  await act(async()=>button.click());
  expect(host.querySelector("details, pre")).toBeNull();
  await act(async()=>host.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  expect(host.querySelector("pre, .agent-message-data-trigger")).toBeNull();
 }finally{await act(async()=>root.unmount());host.remove();}
});
it("keeps failed reads inspectable as ordinary Shell invocations without developer mode",async()=>{
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const row=toolActivityRows([{type:"tool-call",callId:"read",name:"research_read_file",content:'{"path":"missing.md"}'},
  {type:"tool-result",callId:"read",content:'{"error":"file_not_found","message":"file is missing"}',error:true}])[0]!;
 try {
  await act(async()=>root.render(<ToolActivityHistoryRow row={row}/>));
  expect(host.querySelector("details")).not.toBeNull();
  expect(host.querySelector(".tool-row-compact")?.textContent).toBe('已运行 research_read_file {"path":"missing.md"}');
  expect(host.querySelector(".tool-shell-card > header")?.textContent).toBe("Shell");
  expect(host.querySelector('[aria-label="调用结果"]')?.textContent).toContain("file is missing");
  expect(host.querySelectorAll(".tool-shell-card pre")).toHaveLength(1);
  expect(host.querySelector(".tool-shell-exit-code")).toBeNull();
 }finally{await act(async()=>root.unmount());host.remove();}
});
it.each([0,2])("puts the actual exit code %s below the output in one continuous Shell block",async(exitCode)=>{
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const row=toolActivityRows([{type:"tool-call",callId:"cmd",name:"research_exec_command",content:'{"cmd":"check"}'},
  {type:"tool-result",callId:"cmd",content:JSON.stringify({exitCode,stdout:"output\n"}),error:exitCode!==0}])[0]!;
 try {
  await act(async()=>root.render(<ToolActivityHistoryRow row={row}/>));
  expect(host.querySelector("header")?.textContent).toBe("Shell");
  expect(host.querySelectorAll(".tool-shell-card pre")).toHaveLength(1);
  expect(host.querySelector(".tool-shell-content")?.textContent).toBe(`$ check\n\noutput\n\nexit code: ${exitCode}`);
 }finally{await act(async()=>root.unmount());host.remove();}
});

it("renders merged reads as one underlined filename with both original call receipts inline",async()=>{
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const rows=toolActivityRows([0,1].flatMap(i=>[{type:"tool-call" as const,name:"research_read_source",callId:`read-${i}`,content:JSON.stringify({fileId:"file",startChar:i*6})},{type:"tool-result" as const,name:"research_read_source",callId:`read-${i}`,content:JSON.stringify({name:"paper.pdf",sourceUrl:"source://library/paper/file",startChar:i*6,endChar:(i+1)*6,totalChars:12,nextChar:i===0?6:null})}]));
 try{await act(async()=>root.render(<DeveloperModeProvider><DeveloperModeToggle/><ToolActivityHistoryRow row={mergeToolReadRows(rows)[0]!}/></DeveloperModeProvider>));
  expect(host.querySelector(".tool-read-file-name")?.textContent).toBe("paper.pdf");
  expect(host.querySelector(".tool-row-compact")?.textContent).toBe("已读取 paper.pdf");
  await act(async()=>host.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="查看原始数据"]')!.click());
  const data=JSON.parse(host.querySelector("pre")!.textContent!);expect(data.map((part:{call:{callId:string}})=>part.call.callId)).toEqual(["read-0","read-1"]);
 }finally{await act(async()=>root.unmount());host.remove();}
});

it.each([["failed","无法编辑","error"],["declined","拒绝编辑","declined"]] as const)("shows the original %s reason in an edit result box without successful Diff",async(status,title,displayStatus)=>{
 const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
 const receipt=JSON.stringify({status,code:status==="declined"?"edit_declined":"patch_conflict",message:"保留原始理由",operationId:"operation-1",httpStatus:403});
 const row=toolActivityRows([{type:"tool-call",callId:"edit",name:"research_apply_patch",content:JSON.stringify({patch})},{type:"tool-result",callId:"edit",name:"research_apply_patch",content:receipt}])[0]!;
 try{await act(async()=>root.render(<ToolActivityHistoryRow row={row}/>));
 expect(host.querySelector(".tool-row-compact")?.textContent).toBe(`${title} preview.md`);
 expect(host.querySelector("details")?.dataset.editStatus).toBe(status);
 expect(host.querySelector("details")?.dataset.toolStatus).toBe(displayStatus);
 expect(host.querySelector("header")?.textContent).toBe("编辑结果");
 expect(host.querySelector('[aria-label="调用结果"]')?.textContent).toContain("保留原始理由");
 expect(host.querySelector('[aria-label="调用结果"]')?.textContent).toContain("operation-1");
 expect(host.querySelector('[aria-label="调用参数"]')).toBeNull();
 expect(host.querySelector(".tool-patch-diff")).toBeNull();
 expect(host.querySelector(".tool-activity-icon")?.getAttribute("data-tool-kind")).toBe("edit");
 expect(host.querySelector(status==="declined"?".lucide-minus":".lucide-x")).not.toBeNull();
 }finally{await act(async()=>root.unmount());host.remove();}
});
it("shows an old invalid-evidence receipt as rejected and preserves the exact reason and draft metadata",async()=>{
 const host=document.createElement("div"),root=createRoot(host);
 const content=JSON.stringify({status:"failed",targets:[{path:"preview.md",status:"failed",message:"Draft retained",structuredResource:{status:"invalid",resourceId:"synthetic-resource",draftId:"synthetic-draft",draftRevision:1,errors:[{code:"invalid_evidence",message:"Synthetic quote does not match the recorded evidence",path:"$.claims[0].evidence[0].quote"}]}}]});
 const row=toolActivityRows([{type:"tool-call",callId:"validation",name:"research_apply_patch",content:JSON.stringify({patch})},{type:"tool-result",callId:"validation",name:"research_apply_patch",content,error:true}])[0]!;
 try{await act(async()=>root.render(<ToolActivityHistoryRow row={row}/>));
  expect(host.querySelector(".tool-row-compact")?.textContent).toBe("拒绝编辑 preview.md");
  expect(host.querySelector("details")?.dataset.editStatus).toBe("declined");
  expect(host.querySelector(".lucide-minus")).not.toBeNull();expect(host.querySelector(".lucide-x")).toBeNull();
  expect(host.querySelector('[aria-label="调用结果"]')?.textContent).toContain("invalid_evidence");
  expect(host.querySelector('[aria-label="调用结果"]')?.textContent).toContain("synthetic-draft");
  expect(host.querySelector('[aria-label="调用参数"]')).toBeNull();
  expect(host.querySelector(".tool-patch-diff")).toBeNull();
 }finally{await act(async()=>root.unmount());}
});
it("keeps the full failed command exclusively in the developer disclosure",async()=>{
 const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
 const row=toolActivityRows([{type:"tool-call",callId:"private-patch",name:"research_apply_patch",content:JSON.stringify({patch})},{type:"tool-result",callId:"private-patch",name:"research_apply_patch",content:JSON.stringify({status:"failed",code:"permission_denied",message:"Synthetic failure",operationId:"synthetic-operation"})}])[0]!;
 try{await act(async()=>root.render(<DeveloperModeProvider><DeveloperModeToggle/><ToolActivityHistoryRow row={row}/></DeveloperModeProvider>));
  expect(host.querySelector(".tool-shell-content")?.textContent).toContain("Synthetic failure");
  expect(host.querySelector(".tool-shell-content")?.textContent).not.toContain("*** Begin Patch");
  const checkbox=host.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
  if(!checkbox.checked)await act(async()=>checkbox.click());
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="查看原始数据"]')!.click());
  expect(host.querySelector('[aria-label="消息原始数据"]')?.textContent).toContain("*** Begin Patch");
  expect(host.querySelector('[aria-label="消息原始数据"]')?.textContent).toContain("synthetic-operation");
  await act(async()=>checkbox.click());
 }finally{await act(async()=>root.unmount());host.remove();}
});
