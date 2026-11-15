// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {selectAppStorage} from "../runtime/appStorage";
import {selectAppRuntime} from "../runtime/appRuntime";
import {invoke} from "../runtime/platformCore";
import {registerStoragePersistence, type StoragePersistence} from "../runtime/storagePersistence";
import {getHbsDocumentGateway, getPendingHbsDocumentCommit, readHbsDocumentDiagnostics, retryPendingHbsDocumentCommit} from "./hbsDocumentGateway";

vi.mock("../runtime/platformCore", () => ({invoke: vi.fn()}));
const request = {id: "original:1", expectedVersion: 4, saved: {content: "edited candidate"}, baseline: {content: "old baseline"}};
const receipt = {type: "receipt", requestId: request.id, snapshot: {version: 5, saved: request.saved, baseline: request.baseline}};
const key = "research-assistant:hbs-outbox:note:page";
const native = vi.mocked(invoke);
let values: Map<string, string>, durable: Map<string, string>, store: Storage;
let flush: ReturnType<typeof vi.fn<StoragePersistence["flush"]>>;

function selectSnapshot(initial: Map<string, string>): void {
  const records = new Map(initial); values = records;
  store = {get length() {return records.size;}, key: index => [...records.keys()][index] ?? null,
    getItem: key => records.get(key) ?? null, setItem: (key, value) => {records.set(key, value);},
    removeItem: key => {records.delete(key);}, clear: () => records.clear()};
  flush = vi.fn(async () => {durable = new Map(records);});
  registerStoragePersistence(store, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  selectAppRuntime({mode: "desktop", storage: store, gateways: {}, dispose() {}});
}
beforeEach(() => {native.mockReset(); durable = new Map(); selectSnapshot(durable);});
afterEach(() => {vi.restoreAllMocks(); selectAppRuntime({mode:"browser",storage:window.localStorage,gateways:{},dispose(){}});});

it("flushes the exact outbox before invoking and flushes its removal before acknowledging", async () => {
  let release: (() => void) | undefined;
  const waiting = new Promise<void>(resolve => {release = resolve;});
  const events: string[] = [];
  flush.mockImplementationOnce(async () => {events.push("flush requested"); await waiting; durable = new Map(values); events.push("outbox durable");});
  native.mockImplementationOnce(async (command, args) => {
    events.push("invoke");
    expect(command).toBe("hbs_commit_document");
    expect(args).toEqual({kind: "note", resourceId: "page", request});
    expect(JSON.parse(durable.get(key)!)).toEqual(request);
    return receipt;
  });
  const pending = getHbsDocumentGateway().commit("note", "page", request);
  expect(values.get(key)).toBe(JSON.stringify(request)); expect(native).not.toHaveBeenCalled();
  release!();
  await expect(pending).resolves.toEqual(receipt);
  expect(events).toEqual(["flush requested", "outbox durable", "invoke"]);
  expect(flush).toHaveBeenCalledTimes(2); expect(durable.has(key)).toBe(false);
});
it("preserves the safe native error code, message and original request identity", async () => {
  native.mockRejectedValueOnce({code:"MODEL_STORAGE_ERROR",message:"MODEL_STORAGE_ERROR：transaction unavailable",requestId:request.id});
  await expect(getHbsDocumentGateway().commit("note","page",request)).rejects.toMatchObject({
    name:"Error",code:"MODEL_STORAGE_ERROR",message:"MODEL_STORAGE_ERROR：transaction unavailable",requestId:request.id});
  expect(getPendingHbsDocumentCommit("note","page")).toEqual(request);
});

it("does not acknowledge a receipt until removing the pending request is durable", async () => {
  let beginClear = () => {}, finishClear = () => {};
  const clearing = new Promise<void>(resolve => {beginClear = resolve;});
  const complete = new Promise<void>(resolve => {finishClear = resolve;});
  flush.mockImplementationOnce(async () => {durable = new Map(values);})
    .mockImplementationOnce(async () => {beginClear(); await complete; durable = new Map(values);});
  native.mockResolvedValueOnce(receipt);
  let acknowledged = false;
  const pending = getHbsDocumentGateway().commit("note", "page", request).then(value => {acknowledged = true; return value;});
  await clearing;
  expect(acknowledged).toBe(false); expect(durable.has(key)).toBe(true);
  finishClear();
  await expect(pending).resolves.toEqual(receipt);
  expect(acknowledged).toBe(true); expect(durable.has(key)).toBe(false);
});

it("retains an unknown write across reopen and only retries its original request explicitly", async () => {
  const gateway = getHbsDocumentGateway();
  native.mockRejectedValueOnce(Object.assign(new Error("connection lost after dispatch"), {code: "MODEL_STORAGE_ERROR"}));
  await expect(gateway.commit("note", "page", request)).rejects.toThrow("connection lost");
  expect(getPendingHbsDocumentCommit("note", "page")).toEqual(request);
  expect(JSON.parse(durable.get(key)!)).toEqual(request);
  selectSnapshot(durable);
  const reopened = getHbsDocumentGateway();
  native.mockResolvedValueOnce(receipt.snapshot).mockResolvedValueOnce(null);
  await reopened.read("note", "page"); await reopened.readPreservation("note", "page");
  expect(native.mock.calls.map(([command]) => command)).toEqual(["hbs_commit_document", "hbs_read_document", "hbs_read_preservation"]);
  await expect(reopened.commit("note", "page", {...request, id: "new-click:2"})).rejects.toMatchObject({code: "MODEL_REQUEST_UNSETTLED", requestId: request.id});
  expect(native).toHaveBeenCalledTimes(3);
  native.mockResolvedValueOnce(receipt);
  await expect(retryPendingHbsDocumentCommit("note", "page")).resolves.toEqual(receipt);
  expect(native.mock.calls.filter(([command]) => command === "hbs_commit_document").map(([, args]) => args)).toEqual([
    {kind: "note", resourceId: "page", request}, {kind: "note", resourceId: "page", request},
  ]);
  expect(getPendingHbsDocumentCommit("note", "page")).toBeUndefined(); expect(durable.has(key)).toBe(false);
});

it("does not dispatch or acknowledge when the write-ahead flush fails", async () => {
  flush.mockRejectedValueOnce(new Error("IndexedDB blocked"));
  await expect(getHbsDocumentGateway().commit("note", "page", request)).rejects.toThrow("IndexedDB blocked");
  expect(native).not.toHaveBeenCalled(); expect(getPendingHbsDocumentCommit("note", "page")).toEqual(request);
  native.mockResolvedValueOnce(receipt);
  await retryPendingHbsDocumentCommit("note", "page");
  expect(native).toHaveBeenCalledTimes(1); expect(durable.has(key)).toBe(false);
});

it("cancels before dispatch when the runtime owner changes during the outbox flush and retains A's request", async () => {
  const ownerA = store, valuesA = values, durableA = new Map<string, string>();
  let finish = () => {};
  const gate = new Promise<void>(resolve => {finish = resolve;});
  flush.mockImplementationOnce(async () => {await gate; for (const [key, value] of valuesA) durableA.set(key, value);});
  const pending = getHbsDocumentGateway().commit("note", "page", request);
  selectSnapshot(new Map());
  finish();
  await expect(pending).rejects.toMatchObject({code: "MODEL_OWNER_CHANGED"});
  expect(native).not.toHaveBeenCalled();
  expect(JSON.parse(ownerA.getItem(key)!)).toEqual(request);
  expect(JSON.parse(durableA.get(key)!)).toEqual(request);
  expect(getPendingHbsDocumentCommit("note", "page")).toBeUndefined();
  selectAppStorage(ownerA);
  expect(getPendingHbsDocumentCommit("note", "page")).toEqual(request);
});

it("clears a matching CAS conflict while preserving the returned current snapshot", async () => {
  const conflict = {type: "conflict", requestId: request.id, code: "MODEL_VERSION_CONFLICT", snapshot: {...receipt.snapshot, version: 7}};
  native.mockResolvedValueOnce(conflict);
  await expect(getHbsDocumentGateway().commit("note", "page", request)).resolves.toEqual(conflict);
  expect(getPendingHbsDocumentCommit("note", "page")).toBeUndefined(); expect(durable.has(key)).toBe(false);
});

it.each(["MODEL_INVALID_REQUEST", "MODEL_INVALID_DOCUMENT", "MODEL_INVALID_BASELINE", "MODEL_DRAFT_PENDING",
  "MODEL_NOT_FOUND", "MODEL_RESOURCE_INACTIVE", "MODEL_LEGACY_IMPORT_REQUIRED", "MODEL_LEGACY_IMPORT_CONFLICT", "MODEL_REQUEST_TOO_LARGE"])("clears a definite rejection %s with its diagnostic intact", async code => {
  const reason = Object.assign(new Error(`${code}：original safe cause`), {code, requestId: request.id});
  native.mockRejectedValueOnce(reason);
  await expect(getHbsDocumentGateway().commit("note", "page", request)).rejects.toBe(reason);
  expect(getPendingHbsDocumentCommit("note", "page")).toBeUndefined(); expect(durable.has(key)).toBe(false);
});

it.each([
  undefined,
  {type: "receipt", requestId: "someone-else", snapshot: receipt.snapshot},
  {type: "unknown", requestId: request.id, snapshot: receipt.snapshot},
  {type: "receipt", requestId: request.id, snapshot: null},
  {type: "receipt", requestId: request.id, snapshot: {version: "5", saved: request.saved, baseline: request.baseline}},
  {type: "receipt", requestId: request.id, snapshot: {version: -1, saved: request.saved, baseline: request.baseline}},
  {type: "receipt", requestId: request.id, snapshot: {version: 5, saved: request.saved}},
])("keeps the original outbox when the reply is malformed or unrelated (%#)", async bad => {
  native.mockResolvedValueOnce(bad);
  await getHbsDocumentGateway().commit("note", "page", request);
  expect(getPendingHbsDocumentCommit("note", "page")).toEqual(request);
  expect(JSON.parse(durable.get(key)!)).toEqual(request); expect(flush).toHaveBeenCalledTimes(1);
});

it("does not send anything when an explicit retry has no pending request", async () => {
  expect(await retryPendingHbsDocumentCommit("structured", "resource")).toBeUndefined();
  expect(native).not.toHaveBeenCalled(); expect(flush).not.toHaveBeenCalled();
});

const nativeFailure = {kind:"pureRejected",phase:"validation",outcome:"rolledBack",pluginId:"literature_library",
  code:"invalid_input",packageKey:"a".repeat(64),instanceId:"actual-instance",nativeRequestId:"actual-native-request"};
it.each(["notStarted","rolledBack"])("clears only a matched and definitely settled pure rejection (%s)",async outcome=>{
  native.mockRejectedValueOnce({code:"MODEL_INVALID_DOCUMENT",message:"MODEL_INVALID_DOCUMENT：$.fields 无效",requestId:request.id,
    nativeFailure:{...nativeFailure,outcome}});
  const error=await getHbsDocumentGateway().commit("structured","resource",request).catch((reason:unknown)=>reason);
  expect(error).toMatchObject({code:"MODEL_INVALID_DOCUMENT",requestId:request.id,nativeFailure:{...nativeFailure,outcome}});
  expect((error as Error).message).toContain(`requestId=${request.id}`);
  expect((error as Error).message).toContain("nativeRequestId=actual-native-request");
  expect(getPendingHbsDocumentCommit("structured","resource")).toBeUndefined();
  expect(durable.has("research-assistant:hbs-outbox:structured:resource")).toBe(false);
});

it.each([
  {...nativeFailure,outcome:"unknown"},
  {...nativeFailure,outcome:"committed"},
  {...nativeFailure,kind:"technical",code:"plugin_host_exited",phase:"postcommit-read"},
  {...nativeFailure,kind:"sourceData",code:"plugin_data_sql_failed",dataError:{code:"plugin_data_sql_failed",phase:"read",safeCause:"ConstraintViolation"}},
  {...nativeFailure,nativeRequestId:undefined},
  {...nativeFailure,phase:"future-phase"},
  {...nativeFailure,outcome:"future-outcome"},
  {...nativeFailure,body:"PRIVATE_BODY"},
  null,
])("retains the original request for unknown, technical, Data or damaged Native markers (%#)",async failure=>{
  native.mockRejectedValueOnce({code:"MODEL_INVALID_DOCUMENT",message:"MODEL_INVALID_DOCUMENT：原安全错误",requestId:request.id,nativeFailure:failure});
  await expect(getHbsDocumentGateway().commit("structured","resource",request)).rejects.toThrow("MODEL_INVALID_DOCUMENT：原安全错误");
  expect(getPendingHbsDocumentCommit("structured","resource")).toEqual(request);
  expect(JSON.parse(durable.get("research-assistant:hbs-outbox:structured:resource")!)).toEqual(request);
  expect(flush).toHaveBeenCalledTimes(1);
});

it("keeps a pure rejection with a different Core identity unsettled and retries the original request",async()=>{
  native.mockRejectedValueOnce({code:"MODEL_INVALID_DOCUMENT",message:"MODEL_INVALID_DOCUMENT：原安全错误",requestId:"unrelated-core-request",nativeFailure});
  await expect(getHbsDocumentGateway().commit("structured","resource",request)).rejects.toThrow("unrelated-core-request");
  expect(getPendingHbsDocumentCommit("structured","resource")).toEqual(request);
  native.mockResolvedValueOnce(receipt);
  await retryPendingHbsDocumentCommit("structured","resource");
  expect(native.mock.calls.map(([,args])=>args)).toEqual([
    {kind:"structured",resourceId:"resource",request},{kind:"structured",resourceId:"resource",request}]);
  expect(getPendingHbsDocumentCommit("structured","resource")).toBeUndefined();
});

it("exports only this HBS observer's safe Native/Data diagnostics with distinct request IDs",async()=>{
  const dataError={code:"plugin_data_sql_failed",phase:"read",safeCause:"sqlite_operation_failed",sqliteCode:"ConstraintViolation",sqliteExtendedCode:787,
    packageKey:nativeFailure.packageKey,resourceId:"literature.reading-evidence.latest",resourceVersion:1};
  const terminationError={operation:"group_terminate",kind:"PermissionDenied",rawOsError:1};
  native.mockRejectedValueOnce({code:"MODEL_SOURCE_SAVE_FAILED",message:"MODEL_SOURCE_SAVE_FAILED：读取失败",requestId:request.id,
    nativeFailure:{...nativeFailure,kind:"sourceData",phase:"source-read",code:"plugin_data_sql_failed",dataError:{...dataError,sql:"PRIVATE_SQL",message:"PRIVATE_MESSAGE"},
      databaseError:{code:"storage_error",phase:"read",safeCause:"sqlite_operation_failed",path:"/private/input"},terminationError,
      body:"PRIVATE_BODY",headers:{authorization:"PRIVATE_HEADER"}},path:"/private/input"});
  const error=await getHbsDocumentGateway().commit("structured","resource",request).catch((reason:unknown)=>reason) as Error;
  expect(error.message).toContain("nativeFailure.dataError.resourceId=literature.reading-evidence.latest");
  expect(error.message).toContain("终止阶段诊断（已确认完成收尾）");
  const diagnostic=readHbsDocumentDiagnostics("structured","resource").failures[0];
  expect(diagnostic?.requestId).toBe(request.id);
  expect(diagnostic?.error).toMatchObject({requestId:request.id,nativeFailure:{...nativeFailure,kind:"sourceData",phase:"source-read",code:"plugin_data_sql_failed",
    dataError,databaseError:{code:"storage_error",phase:"read",safeCause:"sqlite_operation_failed"},terminationError}});
  expect(`${error.message} ${JSON.stringify(error)} ${JSON.stringify(diagnostic)}`).not.toMatch(/PRIVATE_|\/private/u);
  expect(error).not.toHaveProperty("cause");expect(getPendingHbsDocumentCommit("structured","resource")).toEqual(request);
});

it("preserves the exact old static object error and definite cleanup without a Native marker",async()=>{
  native.mockRejectedValueOnce({code:"MODEL_INVALID_DOCUMENT",message:"原静态错误：\n保留字节",requestId:request.id,other:"ignored"});
  const error=await getHbsDocumentGateway().commit("structured","resource",request).catch((reason:unknown)=>reason) as Error;
  expect(error.message).toBe("原静态错误：\n保留字节");expect(Object.keys(error)).toEqual(["code","requestId"]);
  expect(getPendingHbsDocumentCommit("structured","resource")).toBeUndefined();
});
