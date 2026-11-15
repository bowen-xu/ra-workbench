import {beforeEach,expect,it,vi} from "vitest";
const ports=vi.hoisted(()=>({paper:vi.fn(),projects:vi.fn(),files:vi.fn(),notes:vi.fn(),reading:vi.fn()}));
vi.mock("@research-assistant/literature-library-plugin/literatureGateway",()=>({getLiteratureGateway:()=>({get:ports.paper})}));
vi.mock("@research-assistant/writing-projects-plugin/writingGateway",()=>({getWritingGateway:()=>({listManuscripts:ports.projects,listFiles:ports.files})}));
vi.mock("../../../core/features/notes/gateways",()=>({getNotesGateway:()=>({listPageMetadata:ports.notes,getReadingNote:ports.reading})}));
import {mockContextRecommendation} from "./mockContextCatalog";
beforeEach(()=>{Object.values(ports).forEach(mock=>mock.mockReset());});
it("resolves a paper by ID without loading any other resource catalogs",async()=>{
  ports.paper.mockResolvedValue({id:"paper",title:"Current title",attachments:[{id:"pdf",mimeType:"application/pdf"}]});
  expect(await mockContextRecommendation({label:"Stale title",resource:{kind:"paper",resourceId:"paper",attachmentId:"pdf"}})).toEqual({label:"Current title",resource:{kind:"paperGroup",resourceId:"paper",groupAccess:"readWrite",newMemberAccess:"readWrite",parts:[]}});
  expect(ports.paper).toHaveBeenCalledExactlyOnceWith("paper");
  expect(ports.projects).not.toHaveBeenCalled();expect(ports.notes).not.toHaveBeenCalled();
});
it("only reads the exact project's files even with more than 60 identically named projects",async()=>{
  ports.projects.mockResolvedValue(Array.from({length:70},(_,id)=>({id:String(id),title:"Same",status:"active"})));
  ports.files.mockResolvedValue([{relativePath:"main.tex",kind:"tex"}]);
  const result=await mockContextRecommendation({label:"Stale",resource:{kind:"manuscriptFile",resourceId:"0",path:"main.tex"}});
  expect(result.resource).toEqual({kind:"manuscript",resourceId:"0",parts:[],groupAccess:"readWrite",newMemberAccess:"readWrite"});
  expect(ports.files).toHaveBeenCalledExactlyOnceWith("0");expect(ports.paper).not.toHaveBeenCalled();expect(ports.notes).not.toHaveBeenCalled();
});
it("recommends an open project without listing or opening any file",async()=>{
  ports.projects.mockResolvedValue([{id:"project",title:"Current project",status:"active"}]);
  ports.files.mockRejectedValue(new Error("No file tab is open"));
  const candidate={label:"Stale",resource:{kind:"manuscript" as const,resourceId:"project",parts:[]}};
  expect(await mockContextRecommendation(candidate)).toEqual({label:"Current project",resource:{kind:"manuscript",resourceId:"project",parts:[],groupAccess:"readWrite",newMemberAccess:"readWrite"}});
  expect(ports.files).not.toHaveBeenCalled();expect(ports.notes).not.toHaveBeenCalled();
  ports.projects.mockResolvedValue([{id:"project",title:"Archived",status:"archived"}]);
  await expect(mockContextRecommendation(candidate)).rejects.toThrow("sourceUnavailable");
});
it("fails when a visible leaf no longer belongs to the object instead of silently granting a fallback",async()=>{
  ports.paper.mockResolvedValue({id:"paper",title:"Current",attachments:[]});
  await expect(mockContextRecommendation({label:"Stale",resource:{kind:"paper",resourceId:"paper",attachmentId:"wrong"}})).rejects.toThrow("sourceUnavailable");
  const stop=new AbortController();stop.abort();
  await expect(mockContextRecommendation({label:"Stale",resource:{kind:"paper",resourceId:"paper"}},stop.signal)).rejects.toMatchObject({name:"AbortError"});
});
