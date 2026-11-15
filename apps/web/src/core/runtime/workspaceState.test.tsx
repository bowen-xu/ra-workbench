// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {beforeEach,expect,it,vi} from "vitest";
import {getAppStorage,selectAppStorage} from "./appStorage";
import {readWorkspaceState,useWorkspaceState,workspaceStringSet,writeWorkspaceState} from "./workspaceState";
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
beforeEach(()=>{localStorage.clear();selectAppStorage(localStorage);});

it("restores typed presentation state, isolates resource keys and keeps disabled embedded views from writing",async()=>{
  const root=createRoot(document.createElement("div"));
  let value!:ReadonlySet<string>,update!:ReturnType<typeof useWorkspaceState<ReadonlySet<string>>>[1];
  function Tree({id,enabled=true}:{id:string;enabled?:boolean}) {
    [value,update]=useWorkspaceState(id,new Set<string>(),workspaceStringSet,enabled);return null;
  }
  writeWorkspaceState("b",["saved"]);
  await act(async()=>root.render(<Tree id="a"/>));
  await act(async()=>update(new Set(["first"])));
  await act(async()=>root.render(<Tree id="b"/>));expect([...value]).toEqual(["saved"]);
  expect([...readWorkspaceState("a",new Set<string>(),workspaceStringSet)]).toEqual(["first"]);
  await act(async()=>root.render(<Tree id="embedded" enabled={false}/>));
  expect(getAppStorage().getItem("research-assistant:workspace.v1:embedded")).toBeNull();
  await act(async()=>root.unmount());
});

it("uses a safe default for corrupt preferences and removes explicitly cleared selections",()=>{
  localStorage.setItem("research-assistant:workspace.v1:broken","{broken");
  expect(readWorkspaceState("broken",new Set<string>(),workspaceStringSet)).toEqual(new Set());
  writeWorkspaceState("selection","id");writeWorkspaceState("selection",undefined);
  expect(localStorage.getItem("research-assistant:workspace.v1:selection")).toBeNull();
});
