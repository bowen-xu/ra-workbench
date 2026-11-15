import{afterEach,expect,it,vi}from"vitest";
import type {PreviewDatabase} from "./previewIndexedDatabase";

function database(): PreviewDatabase {
  return {load:async()=>({revision:0,entries:new Map()}),readRevision:async()=>0,
    commit:async revision=>revision+1,close(){}};
}

afterEach(()=>vi.resetModules());

it("fails closed before application storage assembly instead of opening a partial schema",async()=>{
  vi.resetModules();
  const {openPreviewDatabase}=await import("./previewIndexedDatabase");
  await expect(openPreviewDatabase("synthetic-core-space")).rejects.toMatchObject({code:"preview_storage_unconfigured"});
});

it("retains an explicit test provider when application composition installs its default",async()=>{
  vi.resetModules();
  const {openPreviewDatabase,installPreviewDatabaseFactory,configurePreviewDatabase}=await import("./previewIndexedDatabase");
  const test=vi.fn(async(_name:string)=>database()),application=vi.fn(async(_name:string)=>database());
  configurePreviewDatabase(test);
  installPreviewDatabaseFactory(application);
  await openPreviewDatabase("synthetic-test-space");
  expect(test).toHaveBeenCalledWith("synthetic-test-space");
  expect(application).not.toHaveBeenCalled();
});
