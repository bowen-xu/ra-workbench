import {expect,it} from "vitest";
import {toolActivityRows} from "./toolActivityModel";
import {toolDiscoveryDescription,toolDiscoveryText} from "./toolDiscoveryActivity";
function row(name:string,args:unknown,result:unknown={}){return toolActivityRows([{type:"tool-call",callId:"a",name,content:JSON.stringify(args)},{type:"tool-result",callId:"a",name,content:JSON.stringify(result)}])[0]!;}
it("omits scope for global searches regardless of hits or receipt metadata",()=>{
 for(const result of [{matches:[]},{matches:[{sourceUrl:"source://one-hit"}]},{scope:{sourceUrl:"source://not-requested",name:"Wrong"}}]){
  const value=row("research_search_sources",{query:"literal query"},result);
  expect(toolDiscoveryText(value)).toBe("已搜索 literal query");expect(toolDiscoveryDescription(value)?.scope).toBeUndefined();
 }
});
it.each(["fileId","sourceId"])("preserves exact zero-match %s scope for shared resource hints",key=>{
 const value=row("research_search_sources",{query:"evidence",[key]:"opaque-handle"},{total:0,matches:[],scope:{sourceUrl:"source://library/paper/file",name:"paper.pdf"}});
 expect(toolDiscoveryDescription(value)?.scope).toEqual({path:"source://library/paper/file",name:"paper.pdf"});
 expect(toolDiscoveryText(value)).toBe("已搜索 evidence（paper.pdf）");
});
it("resolves explicit parent scope by exact object identity without choosing a matching child",()=>{
 const value=row("research_search_sources",{query:"word",resourceId:"source://notes/parent"},{matches:[{sourceUrl:"source://parent/child"}]});
 expect(toolDiscoveryDescription(value)?.scope).toEqual({path:"source://notes/parent"});
});
it.each(["research_find_sources","research_list_sources"])("shows actual discovery criteria and explicit scope for %s",name=>{
 const value=row(name,{resourceId:"source://library/paper",nameQuery:"portrait",pathGlob:"*.json",resourceType:"portrait"});
 expect(toolDiscoveryText(value)).toBe(name.includes("find")?"已查找（资源，名称 portrait，路径 *.json，类型 portrait）":"已列出文件（范围 资源 / 文献画像，名称 portrait，路径 *.json）");
 expect(toolDiscoveryDescription(value)?.scope?.path).toBe("source://library/paper");
});
it("keeps empty list browsing distinct from reading and preserves lifecycle states",()=>{
 const value=row("research_list_sources",{});
 expect(toolDiscoveryText(value)).toBe("已列出文件");
 const search=row("research_search_sources",{query:"word"});
 expect(toolDiscoveryText({...search,status:"running"})).toBe("正在搜索 word");
 expect(toolDiscoveryText({...search,status:"error"})).toBe("搜索失败 word");
 expect(toolDiscoveryText({...search,status:"interrupted"})).toBe("搜索中断 word");
 expect(()=>toolDiscoveryDescription(row("research_search_sources",{query:"word",resourceId:"\u0000"}))).not.toThrow();
});
it.each([["note","所有笔记"],["paper","所有文献"],["manuscriptFile","所有写作项目"]])("names the explicit %s collection without exposing the enum as an ID path",(resourceType,scope)=>{
 const value=row("research_list_sources",{resourceType,offset:20,limit:20},{sources:[],total:0});
 expect(toolDiscoveryText(value)).toBe(`已列出文件（范围 ${scope}）`);
 expect(toolDiscoveryText({...value,status:"running"})).toBe(`正在列出文件（范围 ${scope}）`);
});
it("does not infer a list range from returned children or pagination",()=>{
 expect(toolDiscoveryText(row("research_list_sources",{offset:20,limit:20},{sources:[{resourceId:"one-paper"}],nextOffset:40}))).toBe("已列出文件");
});
