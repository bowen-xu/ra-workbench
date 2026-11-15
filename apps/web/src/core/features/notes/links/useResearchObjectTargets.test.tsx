// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {useResearchObjectTargets} from "./useResearchObjectTargets";
const {list}=vi.hoisted(()=>({list:vi.fn(async()=>[{id:"paper",title:"Paper"}])}));
vi.mock("../../../services/sourceDomainGateways",()=>({getLiteratureGateway:()=>({list})}));
const error=vi.fn();let output:ReturnType<typeof useResearchObjectTargets>;
function Harness({enabled}:{enabled:boolean}){output=useResearchObjectTargets(error,enabled);return null;}
afterEach(()=>{vi.unstubAllGlobals();list.mockClear();error.mockClear();});
it("does not read papers for an empty/hidden workspace and keeps successful candidates across hot returns",async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const root=createRoot(document.createElement("div"));
  await act(async()=>root.render(<Harness enabled={false}/>));expect(list).not.toHaveBeenCalled();
  await act(async()=>root.render(<Harness enabled/>));expect(output).toEqual([{id:"paper",kind:"paper",label:"Paper"}]);
  for(let trip=0;trip<5;trip++){await act(async()=>root.render(<Harness enabled={false}/>));await act(async()=>root.render(<Harness enabled/>));}
  expect(list).toHaveBeenCalledOnce();expect(error).not.toHaveBeenCalled();await act(async()=>root.unmount());
});
