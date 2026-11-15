// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { EMPTY_AGENT_CONTEXT } from "@research-assistant/core/agent";
import { selectAppStorage } from "../../../runtime/appStorage";
import { readConversationContext, subscribeConversationContext, writeConversationContext } from "./contextStorage";

const values=new Map<string,string>();
const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,
  getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},
  removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
afterEach(()=>storage.clear());
it("publishes the saved context only to hosts of the same conversation",()=>{
  selectAppStorage(storage);
  const changed=vi.fn(()=>expect(readConversationContext("current")).toEqual(EMPTY_AGENT_CONTEXT));
  const unsubscribe=subscribeConversationContext("current",changed);
  try {
    writeConversationContext("other",EMPTY_AGENT_CONTEXT);
    expect(changed).not.toHaveBeenCalled();
    writeConversationContext("current",EMPTY_AGENT_CONTEXT);
    expect(changed).toHaveBeenCalledTimes(1);
  } finally {unsubscribe();}
  writeConversationContext("current",EMPTY_AGENT_CONTEXT);
  expect(changed).toHaveBeenCalledTimes(1);
});
