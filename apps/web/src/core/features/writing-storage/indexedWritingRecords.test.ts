// @vitest-environment happy-dom
import {expect,it,vi} from "vitest";
import {indexedWritingRecords} from "./indexedWritingRecords";

it("keeps a stable code and original IndexedDB cause for asynchronous read/write failures",async()=>{
  const transaction=vi.fn(()=>{throw new DOMException("closed synthetic connection","InvalidStateError");});
  const records=indexedWritingRecords({transaction} as unknown as IDBDatabase);
  for(const operation of [()=>records.read([]),()=>records.commit(0,[]),()=>records.noteCatalog!.queryWritingNoteCatalog({category:"all",view:"recent"})])
    await expect(operation()).rejects.toMatchObject({code:"writing_storage_failed",message:expect.stringContaining("InvalidStateError: closed synthetic connection")});
  expect(records.pending).toBe(false);
  records.stop();
  await expect(records.noteCatalog!.queryWritingNoteCatalog({category:"all",view:"recent"})).rejects.toMatchObject({code:"writing_storage_closed"});
});
