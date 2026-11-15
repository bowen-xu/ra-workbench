// @vitest-environment happy-dom
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {selectAppRuntime} from "../../../runtime/appRuntime";
import {saveNoteRecoveryFile} from "./saveNoteRecoveryFile";
const ports=vi.hoisted(()=>({save:vi.fn(),invoke:vi.fn()}));
vi.mock("../../../runtime/platformDialog",()=>({save:ports.save}));
vi.mock("../../../runtime/platformCore",()=>({invoke:ports.invoke}));
beforeEach(()=>{ports.save.mockReset();ports.invoke.mockReset();selectAppRuntime({mode:"desktop",storage:localStorage,gateways:{},dispose(){}});});
afterEach(()=>vi.restoreAllMocks());
it("writes directly to the user-selected file without calling database gateways",async()=>{
  ports.save.mockResolvedValue("/tmp/synthetic-recovery.json");ports.invoke.mockResolvedValue(undefined);
  expect(await saveNoteRecoveryFile("recovery.json",{mock:true})).toBe("saved");
  expect(ports.invoke).toHaveBeenCalledExactlyOnceWith("export_note_recovery_file",{destination:"/tmp/synthetic-recovery.json",content:JSON.stringify({mock:true},null,2)});
});
it("cancellation performs no write and failure retains the structured cause",async()=>{
  ports.save.mockResolvedValue(null);expect(await saveNoteRecoveryFile("x.json",{})).toBe("cancelled");expect(ports.invoke).not.toHaveBeenCalled();
  ports.save.mockResolvedValue("/tmp/x.json");ports.invoke.mockRejectedValue({code:"NOTE_RECOVERY_EXPORT_IO",message:"permission denied",osError:13});
  await expect(saveNoteRecoveryFile("x.json",{})).rejects.toThrow("NOTE_RECOVERY_EXPORT_IO：permission denied; osError=13");
});
