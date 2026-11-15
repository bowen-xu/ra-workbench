import {withSourceNamespaces} from "./sourceDirectoryFixtures";
/** @vitest-environment happy-dom */
import {act,type ReactNode} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {AgentResourcePath} from "./AgentResourcePath";
const directory=vi.hoisted(()=>vi.fn());
vi.mock("./sourceDirectory",()=>({sourceDirectory:directory}));
vi.mock("../../components/ControlHint",()=>({ControlHint:({children,content}:{children:ReactNode;content:ReactNode})=><>{children}<aside>{content}</aside></>}));
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
const cleanups:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const cleanup of cleanups.splice(0))await cleanup();vi.clearAllMocks();});
async function mount(path?:string,childCategory?:string){
 const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
 cleanups.push(async()=>{await act(async()=>root.unmount());host.remove();});
 await act(async()=>root.render(<AgentResourcePath path={path} childCategory={childCategory}/>));return host;
}
it("converts each project ID/path segment to a readable folder without repeating relative prefixes",async()=>{
 const manuscript={title:"论文草稿"};
 const file={value:{kind:"manuscriptFile",manuscript,file:{relativePath:"sections/nested/main.tex"}},child:async()=>undefined};
 const nested={value:{kind:"folder",manuscript,path:"sections/nested"},child:async(id:string)=>id==="main.tex"?file:undefined};
 const folder={value:{kind:"folder",manuscript,path:"sections"},child:async(id:string)=>id==="nested"?nested:undefined};
 const project={value:{kind:"manuscript",manuscript},child:async(id:string)=>id==="sections"?folder:undefined};
 directory.mockReturnValue(withSourceNamespaces({value:{kind:"root"},child:async(id:string)=>id==="private-project-id"?project:undefined}));
 const host=await mount("source://writings/private-project-id/sections/nested/main.tex");
 expect(host.querySelector(".tool-read-file-name")?.textContent).toBe("所有写作项目 / 论文草稿 / sections / nested / main.tex");
 expect(host.textContent).not.toContain("private-project-id");expect(host.textContent).not.toContain("source://");
});
it("keeps the paper scope and its child-note names while adding an explicit category restriction",async()=>{
 const note={value:{kind:"note",snapshot:{page:{title:"实验笔记"}}},child:async()=>undefined};
 directory.mockReturnValue(withSourceNamespaces({value:{kind:"root"},child:async(id:string)=>id==="note-id"?note:undefined}));
 const host=await mount("source://notes/note-id","笔记");
 expect(host.querySelector(".tool-read-file-name")?.textContent).toBe("所有笔记 / 实验笔记 / 笔记");
 expect(host.textContent).not.toContain("paper-id");expect(host.textContent).not.toContain("note-id");
});
it.each(["所有笔记","所有文献","所有写作项目"])("renders the %s virtual category without fabricating an object ID",async category=>{
 const host=await mount(undefined,category);expect(host.querySelector(".tool-read-file-name")?.textContent).toBe(category);
 expect(directory).not.toHaveBeenCalled();
});
it("does not reveal unresolved identities or replace them with guessed names",async()=>{
 directory.mockReturnValue(withSourceNamespaces({value:{kind:"root"},child:async()=>undefined}));
 const host=await mount("source://notes/missing-private-id");
 expect(host.querySelector(".tool-read-file-name")?.textContent).toBe("所有笔记 / 来源不可用");expect(host.textContent).not.toContain("missing-private-id");
});
