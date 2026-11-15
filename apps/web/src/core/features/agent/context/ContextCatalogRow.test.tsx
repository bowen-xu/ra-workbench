// @vitest-environment happy-dom
import {act,useState} from "react";
import {createRoot} from "react-dom/client";
import {expect,it} from "vitest";
import type {AgentContextSelection} from "@research-assistant/core/agent";
import {ContextCatalogRow} from "./ContextCatalogRow";

it("locks all child access under a read-only group and restores individual choices",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const item:AgentContextSelection={label:"Paper",resource:{kind:"paperGroup",resourceId:"paper",groupAccess:"readWrite",newMemberAccess:"readWrite",parts:[
    {resource:{kind:"paper",resourceId:"paper",attachmentId:"pdf"},label:"PDF",selected:true,access:"readOnly"},
    {resource:{kind:"paperNotes",resourceId:"paper",excludedNoteIds:[]},label:"Notes",selected:true,access:"readWrite"},
    {resource:{kind:"paperPortrait",resourceId:"paper"},label:"Portrait",selected:true,access:"readWrite"},
  ]}};
  function Harness(){const [saved,setSaved]=useState<AgentContextSelection|null>(null);return <ul><ContextCatalogRow item={item} {...(saved?{selected:saved}:{})} disabled={false} onChange={setSaved}/></ul>;}
  const box=(name:string)=>host.querySelector<HTMLInputElement>(`[aria-label="${name}"]`)!;
  try {
    await act(async()=>root.render(<Harness/>));
    await act(async()=>box("使用 Paper").click());
    expect(box("Paper 文件组权限").checked).toBe(true);
    await act(async()=>box("Paper · Portrait 权限").click());
    await act(async()=>box("Paper 文件组权限").click());
    for(const label of ["PDF","Notes","Portrait"]) {
      expect(box(`Paper · ${label} 权限`).checked).toBe(false);
      expect(box(`Paper · ${label} 权限`).disabled).toBe(true);
    }
    await act(async()=>root.render(<Harness/>));
    await act(async()=>box("Paper 文件组权限").click());
    expect(box("Paper · Notes 权限").checked).toBe(true);
    expect(box("Paper · Portrait 权限").checked).toBe(false);
    expect(box("Paper · PDF 权限").disabled).toBe(true);
  } finally {await act(async()=>root.unmount());host.remove();}
});

it("parent grants inherit new members while exclusions survive inventory removal and restoration",async()=>{
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const file=(id:string)=>({resource:{kind:"paper" as const,resourceId:"paper",attachmentId:id},label:id,selected:true,access:"readOnly" as const});
  let inventory:AgentContextSelection={label:"Paper",resource:{kind:"paperGroup",resourceId:"paper",newMemberAccess:"readOnly",parts:[file("old.pdf")]}};
  let latest:AgentContextSelection|null=null;
  function Harness(){const [saved,setSaved]=useState<AgentContextSelection|null>(null);return <ul><ContextCatalogRow item={inventory} {...(saved ? {selected:saved} : {})} disabled={false} onChange={value=>{latest=value;setSaved(value);}}/></ul>;}
  const box=(label:string)=>host.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
  try {
    await act(async()=>root.render(<Harness/>));
    await act(async()=>box("使用 Paper").click());
    expect(latest).toMatchObject({resource:{newMemberAccess:"readWrite",groupAccess:"readWrite"}});
    await act(async()=>box("使用 Paper · old.pdf").click());
    inventory={...inventory,resource:{kind:"paperGroup",resourceId:"paper",newMemberAccess:"readOnly",parts:[file("new.pdf")]}};
    await act(async()=>root.render(<Harness/>));
    expect(box("使用 Paper · new.pdf").checked).toBe(true);
    await act(async()=>box("使用 Paper · new.pdf").click());
    inventory={...inventory,resource:{...inventory.resource,parts:[file("old.pdf"),file("new.pdf"),file("later.pdf")]} as AgentContextSelection["resource"]};
    await act(async()=>root.render(<Harness/>));
    expect(box("使用 Paper · old.pdf").checked).toBe(false);
    expect(box("使用 Paper · new.pdf").checked).toBe(false);
    expect(box("使用 Paper · later.pdf").checked).toBe(true);
    // Parent clear revokes the whole scope, including future members.
    await act(async()=>box("使用 Paper").click()); // Select all from partial state.
    await act(async()=>box("使用 Paper").click());
    expect(latest).toBeNull();
  } finally {await act(async()=>root.unmount());host.remove();}
});

it("sets a project's default access for future files while a child exclusion remains effective",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  let inventory:AgentContextSelection={label:"Project",resource:{kind:"manuscript",resourceId:"project",groupAccess:"readWrite",newMemberAccess:"readWrite",parts:[{resource:{kind:"manuscriptFile",resourceId:"project",path:"main.tex"},label:"main.tex",selected:true,access:"readWrite"}]}};
  let latest:AgentContextSelection|null=null;
  function Harness(){const [saved,setSaved]=useState<AgentContextSelection|null>(null);return <ul><ContextCatalogRow item={inventory} {...(saved ? {selected:saved} : {})} disabled={false} onChange={value=>{latest=value;setSaved(value);}}/></ul>;}
  const box=(label:string)=>host.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
  try {
    await act(async()=>root.render(<Harness/>));
    await act(async()=>box("使用 Project").click());
    const access=box("Project 文件组权限");
    expect(access.getAttribute("role")).toBe("switch");
    await act(async()=>access.click());
    expect(box("Project · main.tex 权限").disabled).toBe(true);
    await act(async()=>access.click());
    expect(latest).toMatchObject({resource:{newMemberAccess:"readWrite"}});
    await act(async()=>box("使用 Project · main.tex").click());
    inventory={...inventory,resource:{kind:"manuscript",resourceId:"project",parts:[{resource:{kind:"manuscriptFile",resourceId:"project",path:"main.tex"},label:"main.tex",selected:true,access:"readOnly"},{resource:{kind:"manuscriptFile",resourceId:"project",path:"new.tex"},label:"new.tex",selected:true,access:"readOnly"}]}};
    await act(async()=>root.render(<Harness/>));
    expect(box("使用 Project · main.tex").checked).toBe(false);
    expect(box("使用 Project · new.tex").checked).toBe(true);
    expect(box("Project · new.tex 权限").checked).toBe(true);
  } finally {await act(async()=>root.unmount());host.remove();}
});

it("uses switches for access, keeps PDF read-only, and preserves saved permissions over new defaults",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const item:AgentContextSelection={label:"Paper",resource:{kind:"paperGroup",resourceId:"paper",newMemberAccess:"readOnly",parts:[
    {resource:{kind:"paper",resourceId:"paper",attachmentId:"pdf"},label:"PDF",selected:true,access:"readOnly"},
    {resource:{kind:"paperNotes",resourceId:"paper",excludedNoteIds:[]},label:"Notes",selected:true,access:"readWrite"},
    {resource:{kind:"paperPortrait",resourceId:"paper"},label:"Portrait",selected:true,access:"readWrite"},
  ]}};
  let latest:AgentContextSelection|null=null;
  function Harness(){const [saved,setSaved]=useState<AgentContextSelection|null>(null);return <ul><ContextCatalogRow item={item} {...(saved?{selected:saved}:{})} disabled={false} onChange={value=>{latest=value;setSaved(value);}}/></ul>;}
  const box=(label:string)=>host.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
  try {
    await act(async()=>root.render(<Harness/>));
    await act(async()=>box("使用 Paper").click());
    expect(host.querySelectorAll('select')).toHaveLength(0);
    expect(box("Paper · PDF 权限").checked).toBe(false);
    expect(box("Paper · PDF 权限").disabled).toBe(true);
    expect(box("Paper · Notes 权限").checked).toBe(true);
    expect(box("Paper · Portrait 权限").checked).toBe(true);
    await act(async()=>box("Paper · Notes 权限").click());
    expect(latest).toMatchObject({resource:{parts:[{}, {access:"readOnly"}, {}]}});
    await act(async()=>root.render(<Harness/>));
    expect(box("Paper · Notes 权限").checked).toBe(false);
    expect(box("Paper · Portrait 权限").checked).toBe(true);
    await act(async()=>box("Paper · Notes 权限").click());
    expect(box("Paper · Notes 权限").checked).toBe(true);
  } finally {await act(async()=>root.unmount());host.remove();}
});

it("can authorize and revoke an empty managed project",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const item:AgentContextSelection={label:"Empty",resource:{kind:"manuscript",resourceId:"empty",parts:[],newMemberAccess:"readOnly"}};
  function Harness(){const [saved,setSaved]=useState<AgentContextSelection|null>(null);return <ul><ContextCatalogRow item={item} {...(saved ? {selected:saved} : {})} disabled={false} onChange={setSaved}/></ul>;}
  try {
    await act(async()=>root.render(<Harness/>));
    const box=()=>host.querySelector<HTMLInputElement>('[aria-label="使用 Empty"]')!;
    await act(async()=>box().click());expect(box().checked).toBe(true);
    await act(async()=>box().click());expect(box().checked).toBe(false);
  } finally {await act(async()=>root.unmount());host.remove();}
});

it("keeps the group writable when the only selected child becomes read-only",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const item:AgentContextSelection={label:"Note",resource:{kind:"noteGroup",resourceId:"note",groupAccess:"readWrite",newMemberAccess:"readWrite",parts:[{resource:{kind:"note",resourceId:"note"},label:"Body",selected:true,access:"readWrite"}]}};
  function Harness(){const [saved,setSaved]=useState<AgentContextSelection|null>(null);return <ul><ContextCatalogRow item={item} {...(saved?{selected:saved}:{})} disabled={false} onChange={setSaved}/></ul>;}
  const box=(label:string)=>host.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
  try {
    await act(async()=>root.render(<Harness/>));
    await act(async()=>box("使用 Note · Body").click());
    expect(box("Note 文件组权限").checked).toBe(true);
    await act(async()=>box("Note · Body 权限").click());
    expect(box("Note 文件组权限").checked).toBe(true);
    expect(box("Note · Body 权限").checked).toBe(false);
    expect(box("Note · Body 权限").disabled).toBe(false);
    await act(async()=>box("Note · Body 权限").click());
    expect(box("Note · Body 权限").checked).toBe(true);
  } finally {await act(async()=>root.unmount());host.remove();}
});
