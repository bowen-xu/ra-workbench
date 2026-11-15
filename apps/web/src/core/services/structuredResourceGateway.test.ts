// @vitest-environment happy-dom
import { afterEach,expect,it,vi } from "vitest";
import { selectAppRuntime } from "../runtime/appRuntime";
import { invoke } from "../runtime/platformCore";
import { getStructuredResourceGateway, structuredWriteValidationDiagnostic } from "./structuredResourceGateway";
import { hbsDocumentError, hbsNativeDiagnostic } from "./hbsNativeFailure";
import type { StructuredWriteResult } from "@research-assistant/core/structured-resources";
vi.mock("../runtime/platformCore",()=>({invoke:vi.fn(),isDesktopRuntime:()=>true}));
const values=new Map<string,string>();
const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
afterEach(()=>{storage.clear();vi.clearAllMocks();});
it("settles an uncertain write with its original request identity before acknowledging a retry",async()=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  const resource={id:"object",kind:"portrait",paperId:"paper",sourceUri:"source://library/paper/object",title:"画像",revision:1,content:null,reviewBaseline:null,pendingFieldIds:[],reviewedFieldIds:[]};
  const result={status:"applied",resource,errors:[],receipt:{requestId:"original",resourceId:"object",revision:1,contentHash:"hash"}};
  const native=vi.mocked(invoke);native.mockRejectedValueOnce(new Error("connection lost")).mockResolvedValueOnce(result);
  const request={resourceId:"object",expectedRevision:0,requestId:"original",content:"{}",editorKind:"human" as const};
  const gateway=getStructuredResourceGateway();await expect(gateway.write(request)).rejects.toThrow("connection lost");
  expect(values.size).toBe(1);
  await expect(gateway.write({...request,requestId:"new-click"})).resolves.toEqual(result);
  expect(native.mock.calls.map(call=>(call[1] as {draft:{requestId:string}}).draft.requestId)).toEqual(["original","original"]);expect(values.size).toBe(0);
});

it("passes all startup selections and raw context caches to the native atomic migration contract", async () => {
  selectAppRuntime({mode: "desktop", storage, gateways: {}, dispose() {}});
  const draft = {presentations: [{paperId: "paper", presentation: {fields: [{candidateId: "older", fieldId: "q"}], hiddenUnitIds: []}}],
    contexts: [{conversationId: "conversation", state: "corrupt cache already superseded by native"}]};
  const summary = {paperIds: ["paper"], importedPaperCount: 1, importedTemplateCount: 0, importedContextCount: 0};
  vi.mocked(invoke).mockResolvedValueOnce(summary);
  await expect(getStructuredResourceGateway().importStartup(draft)).resolves.toEqual(summary);
  expect(invoke).toHaveBeenCalledWith("structured_resource_import_startup", {draft});
});

it.each([
  {deferredPaperIds: ["paper", "other-paper"], deferredContextIds: ["conversation"]},
  {deferredPaperIds: []},
  {deferredContextIds: []},
])("preserves optional deferred startup identities without filling absent fields: %j", async deferred => {
  selectAppRuntime({mode: "desktop", storage, gateways: {}, dispose() {}});
  const summary = {paperIds: [], importedPaperCount: 0, importedTemplateCount: 0, ...deferred};
  vi.mocked(invoke).mockResolvedValueOnce(summary);
  await expect(getStructuredResourceGateway().importStartup({presentations: [], contexts: []})).resolves.toBe(summary);
});

it.each(["deferredPaperIds", "deferredContextIds"] as const)("rejects non-string-array %s startup identities", async key => {
  selectAppRuntime({mode: "desktop", storage, gateways: {}, dispose() {}});
  for (const invalid of [null, "paper", ["paper", 7], {}]) {
    vi.mocked(invoke).mockResolvedValueOnce({paperIds: [], importedPaperCount: 0, importedTemplateCount: 0, [key]: invalid});
    await expect(getStructuredResourceGateway().importStartup({presentations: [], contexts: []})).rejects.toThrow("启动迁移回执无效");
  }
});

const sourceCalls = [
  {command: "structured_resource_import_startup", run: () => getStructuredResourceGateway().importStartup({presentations: [], contexts: []})},
  {command: "structured_import_legacy", run: () => getStructuredResourceGateway().importLegacy({paperId: "paper", requestId: "legacy-request"})},
  {command: "structured_get_resource_sources", run: () => getStructuredResourceGateway().getSources("reading")},
];
it.each(sourceCalls)("retains finite Native source diagnostics through the real $command gateway", async ({command, run}) => {
  selectAppRuntime({mode: "desktop", storage, gateways: {}, dispose() {}});
  const packageKey = "a".repeat(64), terminationError = {operation: "group_terminate", kind: "PermissionDenied", rawOsError: 1};
  const dataError = {code: "plugin_data_sql_failed", phase: "read", packageKey, resourceId: "literature.reading-evidence.snapshot-rows",
    resourceVersion: 1, safeCause: "ConstraintViolation", sqliteCode: "ConstraintViolation", sqliteExtendedCode: 787};
  vi.mocked(invoke).mockRejectedValueOnce({code: "invalid_input", message: "Original source read failed", pluginId: "literature_library",
    packageKey, instanceId: "source-instance", requestId: "actual-initialize-request", dataError: {...dataError, sql: "PRIVATE_SQL", body: "PRIVATE_BODY", message: "PRIVATE_DATA_MESSAGE"},
    terminationError, headers: {authorization: "PRIVATE_HEADER"}, path: "/private/source", cleanupError: "/private/cleanup", unknown: "PRIVATE_UNKNOWN"});
  const error = await run().catch((reason: unknown) => reason) as Error & Record<string, unknown>;
  expect(invoke).toHaveBeenCalledWith(command, expect.any(Object));
  expect(error).toBeInstanceOf(Error); expect(error.code).toBe("invalid_input");
  expect(error.packageKey).toBe(packageKey); expect(error.instanceId).toBe("source-instance"); expect(error.requestId).toBe("actual-initialize-request");
  expect(error.dataError).toEqual(dataError); expect(error.terminationError).toEqual(terminationError);
  expect(error.message).toContain("Original source read failed"); expect(error.message).toContain("code=invalid_input");
  expect(error.message).toContain("dataError.resourceVersion=1"); expect(error.message).toContain("dataError.sqliteExtendedCode=787");
  expect(error.message).toContain("终止阶段诊断（已确认完成收尾）");
  expect(`${error.message} ${JSON.stringify(error)}`).not.toMatch(/PRIVATE_|\/private/u); expect(error).not.toHaveProperty("cause");
});

it.each(sourceCalls)("preserves exact old static code and message for $command", async ({run}) => {
  selectAppRuntime({mode: "desktop", storage, gateways: {}, dispose() {}});
  vi.mocked(invoke).mockRejectedValueOnce({code: "invalid_input", message: "原静态错误：\n保留字节", dataError: {code: "ignored"}});
  const error = await run().catch((reason: unknown) => reason);
  expect(error).toMatchObject({code: "invalid_input", message: "原静态错误：\n保留字节"});
  expect(Object.keys(error as Error)).toEqual(["code"]);
});

it("does not invent source request IDs or copy unsafe, unknown or malformed diagnostics", async () => {
  selectAppRuntime({mode: "desktop", storage, gateways: {}, dispose() {}});
  vi.mocked(invoke).mockRejectedValueOnce({code: "storage_error", message: "Source failure", pluginId: "literature_library", packageKey: "../private/key",
    instanceId: "bad instance", dataError: {code: "plugin_data_sql_failed", phase: "read", resourceId: "/private/path", resourceVersion: 0,
      safeCause: "SELECT body FROM private", sqliteCode: {body: "PRIVATE_BODY"}, sqliteExtendedCode: Number.MAX_SAFE_INTEGER},
    terminationError: {operation: "group_terminate", kind: "PermissionDenied", rawOsError: 1, body: "PRIVATE_BODY"}});
  const error = await getStructuredResourceGateway().getSources("reading").catch((reason: unknown) => reason) as Error & Record<string, unknown>;
  expect(error).not.toHaveProperty("requestId"); expect(error).not.toHaveProperty("instanceId"); expect(error).not.toHaveProperty("packageKey");
  expect(error).not.toHaveProperty("terminationError"); expect(error.dataError).toEqual({code: "plugin_data_sql_failed", phase: "read"});
  expect(`${error.message} ${JSON.stringify(error)}`).not.toMatch(/SELECT|private|PRIVATE|requestId=/u);
});

it("retains only the SDK's fixed shape cause grammar and leaves unrelated commands unchanged", async () => {
  selectAppRuntime({mode: "desktop", storage, gateways: {}, dispose() {}});
  const reason = {code: "invalid_input", message: "Original source failure", pluginId: "literature_library", packageKey: "a".repeat(64),
    dataError: {safeCause: "invalid data value at row 2, column 1: data value does not have the declared Text type"}};
  vi.mocked(invoke).mockRejectedValueOnce(reason).mockRejectedValueOnce(reason);
  await expect(getStructuredResourceGateway().getSources("reading")).rejects.toThrow(`dataError.safeCause=${reason.dataError.safeCause}`);
  const old = await getStructuredResourceGateway().list("paper").catch((error: unknown) => error);
  expect(old).toMatchObject({code: reason.code, message: reason.message}); expect(Object.keys(old as Error)).toEqual(["code"]);
});

const reviewResource={id:"object",kind:"structuredReading",paperId:"paper",sourceUri:"source://library/paper/object",title:"阅读",revision:2,
  content:null,reviewBaseline:null,pendingFieldIds:[],reviewedFieldIds:[]};
const finiteReviewCalls=[
  {command:"structured_review_resource",draft:{resourceId:"object",expectedRevision:1,requestId:"original-core",action:"keep" as const,scope:"unreviewed" as const},
    run:(requestId="original-core")=>getStructuredResourceGateway().review({resourceId:"object",expectedRevision:1,requestId,action:"keep",scope:"unreviewed"})},
  {command:"structured_restore_resource_history",draft:{resourceId:"object",expectedRevision:1,requestId:"original-core",operationRevision:1,direction:"undo" as const},
    run:(requestId="original-core")=>getStructuredResourceGateway().restoreHistory({resourceId:"object",expectedRevision:1,requestId,operationRevision:1,direction:"undo"})},
];
const matchedFailure={kind:"pureRejected",phase:"validation",outcome:"rolledBack",pluginId:"literature_library",code:"invalid_input",
  packageKey:"a".repeat(64),instanceId:"actual-instance",nativeRequestId:"actual-native-request"};
const pendingReviewKey=(command:string)=>`research-assistant:structured-request:${command}:object`;

it.each(finiteReviewCalls)("keeps safe Native diagnostics and independent identities only for $command",async({command,run,draft})=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  const dataError={code:"plugin_data_sql_failed",phase:"write",safeCause:"sqlite_operation_failed",sqliteCode:"ConstraintViolation",sqliteExtendedCode:787,
    packageKey:matchedFailure.packageKey,resourceId:"literature.reading-evidence.validation-latest",resourceVersion:1};
  const terminationError={operation:"group_terminate",kind:"PermissionDenied",rawOsError:1};
  vi.mocked(invoke).mockRejectedValueOnce({code:"invalid_input",message:"原安全数据失败",requestId:draft.requestId,
    nativeFailure:{...matchedFailure,kind:"sourceData",phase:"write",outcome:"committed",code:"plugin_data_sql_failed",
      dataError:{...dataError,message:"PRIVATE_MESSAGE",sql:"PRIVATE_SQL",body:"PRIVATE_BODY"},terminationError,
      databaseError:{code:"storage_error",phase:"write",safeCause:"sqlite_operation_failed",path:"/private/input"},
      cleanupError:"PRIVATE_CLEANUP",headers:{authorization:"PRIVATE_HEADER"}}});
  const error=await run().catch((reason:unknown)=>reason) as Error & Record<string,unknown>;
  expect(error).toBeInstanceOf(Error);expect(error.requestId).toBe(draft.requestId);
  expect(error.message).toContain(`requestId=${draft.requestId}`);expect(error.message).toContain("nativeRequestId=actual-native-request");
  expect(error.message).toContain("dataError.phase=write");expect(error.message).toContain("sqliteExtendedCode=787");
  expect(error.message).toContain("终止阶段诊断（已确认完成收尾）");
  expect(hbsNativeDiagnostic(error)).toMatchObject({nativeFailure:{kind:"sourceData",dataError,terminationError}});
  expect(`${error.message} ${JSON.stringify(error)} ${JSON.stringify(hbsNativeDiagnostic(error))}`).not.toMatch(/PRIVATE_|\/private/u);
  expect(error).not.toHaveProperty("cause");expect(JSON.parse(storage.getItem(pendingReviewKey(command))!)).toEqual(draft);
});

it.each(finiteReviewCalls.flatMap(call=>["notStarted","rolledBack"].map(outcome=>({...call,outcome}))))(
  "clears the original $command only for a matched settled pure rejection ($outcome)",async({command,run,outcome})=>{
    selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
    vi.mocked(invoke).mockRejectedValueOnce({code:"invalid_input",message:"原领域拒绝：\n保留文案",requestId:"original-core",nativeFailure:{...matchedFailure,outcome}});
    const error=await run().catch((reason:unknown)=>reason) as Error;
    expect(error.message).toContain("原领域拒绝：\n保留文案");expect(storage.getItem(pendingReviewKey(command))).toBeNull();
  });

const unsettledMarkers=[
  {...matchedFailure,outcome:"unknown"},
  {...matchedFailure,outcome:"committed"},
  {...matchedFailure,kind:"technical",code:"plugin_host_exited"},
  {...matchedFailure,kind:"sourceData",code:"plugin_data_sql_failed"},
  {...matchedFailure,phase:"future-phase"},
  {...matchedFailure,outcome:"future-outcome"},
  {...matchedFailure,nativeRequestId:undefined},
  {...matchedFailure,packageKey:"../private/key"},
  {...matchedFailure,rollbackError:{code:"rollback_failed",phase:"rollback",safeCause:"sqlite_operation_failed"}},
  {...matchedFailure,body:"PRIVATE_BODY"},
  undefined,null,[],{},
];
it.each(finiteReviewCalls.flatMap(call=>unsettledMarkers.map(nativeFailure=>({...call,nativeFailure}))))(
  "retains the exact pending $command for technical, unknown or damaged markers (%#)",async({command,run,draft,nativeFailure})=>{
    selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
    vi.mocked(invoke).mockRejectedValueOnce({code:"invalid_input",message:"原错误",requestId:draft.requestId,nativeFailure});
    await expect(run()).rejects.toThrow("原错误");expect(JSON.parse(storage.getItem(pendingReviewKey(command))!)).toEqual(draft);
  });

it.each(finiteReviewCalls)("retries the exact persisted $command identity after a mismatched or twice-adapted failure",async({command,run,draft})=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  const damaged=hbsDocumentError({code:"invalid_input",message:"原安全错误",requestId:draft.requestId,nativeFailure:{...matchedFailure,body:"PRIVATE_BODY"}});
  vi.mocked(invoke).mockRejectedValueOnce({code:"invalid_input",message:"无关Core身份",requestId:"unrelated-core",nativeFailure:matchedFailure})
    .mockRejectedValueOnce(hbsDocumentError(damaged)).mockResolvedValueOnce(reviewResource);
  await expect(run()).rejects.toThrow("unrelated-core");await expect(run("second-click")).rejects.toThrow("原安全错误");
  expect(JSON.parse(storage.getItem(pendingReviewKey(command))!)).toEqual(draft);
  await expect(run("third-click")).resolves.toEqual(reviewResource);
  expect(vi.mocked(invoke).mock.calls.map(([,args])=>(args as {draft:unknown}).draft)).toEqual([draft,draft,draft]);
  expect(storage.getItem(pendingReviewKey(command))).toBeNull();
});

it.each(finiteReviewCalls)("does not invent unavailable Native or Core identities for $command",async({command,run})=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  vi.mocked(invoke).mockRejectedValueOnce({code:"invalid_input",message:"文献包不可用",nativeFailure:{kind:"technical",phase:"sources",outcome:"notStarted",
    pluginId:"literature_library",code:"plugin_package_missing"}});
  const error=await run().catch((reason:unknown)=>reason) as Error;
  expect(error.message).toContain("plugin_package_missing");expect(error.message).not.toMatch(/requestId=|nativeRequestId=|undefined|null/u);
  expect(error).not.toHaveProperty("requestId");expect(storage.getItem(pendingReviewKey(command))).not.toBeNull();
});

it.each(finiteReviewCalls.flatMap(call=>["invalid_input","not_found","revision_conflict"].map(code=>({...call,code}))))(
  "keeps exact legacy static $command projection and definite cleanup ($code)",async({command,run,code})=>{
    selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
    vi.mocked(invoke).mockRejectedValueOnce({code,message:"原静态错误：\n保留字节"});
    const error=await run().catch((reason:unknown)=>reason) as Error;
    expect(error.message).toBe("原静态错误：\n保留字节");expect(Object.keys(error)).toEqual(["code"]);
    expect(storage.getItem(pendingReviewKey(command))).toBeNull();
  });

it.each(finiteReviewCalls.flatMap(call=>["invalid_input","not_found","revision_conflict"].map(code=>({...call,code}))))(
  "shows the actual settled Core identity without inventing Native fields for $command ($code)",async({command,run,draft,code})=>{
    selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
    vi.mocked(invoke).mockRejectedValueOnce({code,message:"原Core明确拒绝：\n保留文案",requestId:draft.requestId});
    const error=await run().catch((reason:unknown)=>reason) as Error & Record<string,unknown>;
    expect(error.message).toBe(`原Core明确拒绝：\n保留文案 · requestId=${draft.requestId}`);
    expect(error.requestId).toBe(draft.requestId);expect(error.code).toBe(code);expect(Object.keys(error)).toEqual(["code","requestId"]);
    expect(error).not.toHaveProperty("nativeFailure");expect(error).not.toHaveProperty("nativeRequestId");expect(hbsNativeDiagnostic(error)).toEqual({});
    expect(storage.getItem(pendingReviewKey(command))).toBeNull();
  });

const coreReply={code:"invalid_input",message:"原fallback错误",requestId:"actual-core"};
const malformedCoreReplies=[
  {...coreReply,requestId:"/private/id"}, {...coreReply,requestId:7}, {...coreReply,code:"unsafe code"},
  {...coreReply,extra:"PRIVATE_EXTRA"}, {...coreReply,[Symbol("unknown")]:"PRIVATE_SYMBOL"},
  Object.assign(new Error(coreReply.message),{code:coreReply.code,requestId:coreReply.requestId}),
  Object.assign([],{...coreReply}),Object.assign(Object.create({unknown:true}) as object,coreReply),
];
it.each(finiteReviewCalls.flatMap(call=>malformedCoreReplies.map(reply=>({...call,reply}))))(
  "keeps the original projection for a non-IPC, extra-field or unsafe Core $command error (%#)",async({command,run,reply})=>{
    selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
    vi.mocked(invoke).mockRejectedValueOnce(reply);
    const error=await run().catch((reason:unknown)=>reason) as Error & Record<string,unknown>;
    expect(error.message).toBe(coreReply.message);expect(Object.keys(error)).toEqual(["code"]);
    expect(error).not.toHaveProperty("requestId");expect(error).not.toHaveProperty("nativeFailure");
    expect(`${error.message} ${JSON.stringify(error)}`).not.toMatch(/PRIVATE_|\/private|requestId=/u);
    expect(storage.getItem(pendingReviewKey(command))===null).toBe(error.code==="invalid_input");
  });

it.each([
  ()=>getStructuredResourceGateway().createTemplate("template","unrelated-create"),
  ()=>getStructuredResourceGateway().bindTemplate({paperId:"paper",templateId:"template",templateRevision:1,expectedRevision:0,confirmed:true,requestId:"unrelated-bind"}),
])("does not expand Native diagnostics or pending cleanup for an unrelated mutation (%#)",async run=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  vi.mocked(invoke).mockRejectedValueOnce({code:"invalid_input",message:"原无关命令错误",requestId:"ignored",nativeFailure:{...matchedFailure,outcome:"unknown"}});
  const error=await run().catch((reason:unknown)=>reason) as Error;
  expect(error.message).toBe("原无关命令错误");expect(Object.keys(error)).toEqual(["code"]);expect(storage.length).toBe(0);
  vi.mocked(invoke).mockRejectedValueOnce({code:"invalid_input",message:"原无关三字段错误",requestId:"ignored"});
  const plain=await run().catch((reason:unknown)=>reason) as Error;
  expect(plain.message).toBe("原无关三字段错误");expect(Object.keys(plain)).toEqual(["code"]);expect(storage.length).toBe(0);
});

const writeRequest={resourceId:"object",expectedRevision:1,requestId:"original-core",content:"{}",editorKind:"human" as const};
const validationOrigin={requestId:writeRequest.requestId,packageKey:"a".repeat(64),instanceId:"actual-instance",nativeRequestId:"actual-native-request"};
const invalidWrite={status:"invalid" as const,resource:reviewResource,errors:[{path:"$.name",code:"invalid_input",message:"原模板拒绝消息"}],
  draft:{id:"draft",resourceId:"object",baseRevision:1,revision:1,content:writeRequest.content,contentHash:"hash",errors:[]}};
const writeKey=pendingReviewKey("structured_write_resource");
const write=(requestId=writeRequest.requestId)=>getStructuredResourceGateway().write({...writeRequest,requestId});

it("acknowledges a normal invalid write and preserves its original repair draft, issues and actual validation identities",async()=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  vi.mocked(invoke).mockResolvedValueOnce({...invalidWrite,validationOrigin});
  const result=await write();
  expect(result).toEqual({...invalidWrite,validationOrigin});expect(storage.getItem(writeKey)).toBeNull();
  expect(structuredWriteValidationDiagnostic(result)).toBe(`requestId=original-core · packageKey=${validationOrigin.packageKey} · instanceId=actual-instance · nativeRequestId=actual-native-request`);
});

it("binds replayed invalid origins to the persisted request rather than a newer click",async()=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  vi.mocked(invoke).mockRejectedValueOnce({code:"storage_error",message:"重试原请求核对草稿",requestId:writeRequest.requestId,
    nativeFailure:{...matchedFailure,kind:"technical",phase:"commit",outcome:"unknown",code:"storage_error"}})
    .mockResolvedValueOnce({...invalidWrite,validationOrigin});
  await expect(write()).rejects.toThrow("重试原请求核对草稿");
  expect(JSON.parse(storage.getItem(writeKey)!)).toEqual(writeRequest);
  const result=await write("new-click");
  expect(result.validationOrigin).toEqual(validationOrigin);expect(structuredWriteValidationDiagnostic(result)).toContain("requestId=original-core");
  expect(vi.mocked(invoke).mock.calls.map(([,args])=>(args as {draft:unknown}).draft)).toEqual([writeRequest,writeRequest]);
  expect(storage.getItem(writeKey)).toBeNull();
});

it.each([
  {...validationOrigin,requestId:"another-request"}, {...validationOrigin,requestId:"/private/request"},
  {...validationOrigin,packageKey:"PRIVATE_KEY"}, {...validationOrigin,instanceId:"/private/instance"},
  {...validationOrigin,nativeRequestId:"PRIVATE BODY"},
  null, [], {}, "PRIVATE_BODY", {...validationOrigin,instanceId:7}, {...validationOrigin,extra:"PRIVATE_BODY"},
  {...validationOrigin,[Symbol("unknown")]:"PRIVATE_BODY"},
])("drops unsafe or mismatched normal validation origins without discarding an acknowledged invalid draft (%#)",async origin=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  vi.mocked(invoke).mockResolvedValueOnce({...invalidWrite,validationOrigin:origin});
  const result=await write();expect(result).toEqual(invalidWrite);expect(structuredWriteValidationDiagnostic(result)).toBe("");
  expect(`${JSON.stringify(result)} ${structuredWriteValidationDiagnostic(result)}`).not.toMatch(/PRIVATE|\/private/u);
  expect(storage.getItem(writeKey)).toBeNull();
});

it.each(["conflict","applied"])("never presents an origin on a non-rejection %s result",async status=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  vi.mocked(invoke).mockResolvedValueOnce({...invalidWrite,status,validationOrigin,
    ...(status==="applied"?{receipt:{requestId:writeRequest.requestId,resourceId:"object",revision:reviewResource.revision,contentHash:"hash"}}:{})});
  const result=await write();expect(result).not.toHaveProperty("validationOrigin");expect(structuredWriteValidationDiagnostic(result)).toBe("");
});

it.each([
  null, {...validationOrigin,packageKey:"PRIVATE_KEY"}, {...validationOrigin,instanceId:"/private/id"},
  {...validationOrigin,extra:"PRIVATE_BODY"}, {...validationOrigin,[Symbol("extra")]:"PRIVATE_BODY"},
  Object.assign(Object.create({private:true}) as object,validationOrigin),
])("does not print a tampered origin from a typed preview gateway (%#)",origin=>{
  expect(structuredWriteValidationDiagnostic({...invalidWrite,validationOrigin:origin} as unknown as StructuredWriteResult)).toBe("");
});

it.each(["invalid_input","not_found","revision_conflict"])("keeps original static write errors and cleanup (%s)",async code=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  vi.mocked(invoke).mockRejectedValueOnce({code,message:"原静态错误：\n保留字节"});
  const error=await write().catch((reason:unknown)=>reason) as Error;
  expect(error.message).toBe("原静态错误：\n保留字节");expect(Object.keys(error)).toEqual(["code"]);expect(storage.getItem(writeKey)).toBeNull();
});

it.each(["invalid_input","not_found","revision_conflict"])("retains the actual matched Core write rejection identity (%s)",async code=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  vi.mocked(invoke).mockRejectedValueOnce({code,message:"原明确业务拒绝",requestId:writeRequest.requestId});
  const error=await write().catch((reason:unknown)=>reason) as Error;
  expect(error.message).toBe(`原明确业务拒绝 · requestId=${writeRequest.requestId}`);
  expect(error).not.toHaveProperty("nativeFailure");expect(storage.getItem(writeKey)).toBeNull();
});

it.each([
  ...malformedCoreReplies, {code:"invalid_input",message:"原fallback错误",requestId:"other-core"},
  {code:"invalid_input"}, {code:"invalid_input",message:7,requestId:writeRequest.requestId,nativeFailure:matchedFailure},
  ...["sourceData","technical"].flatMap(kind=>["notStarted","rolledBack","committed","unknown"].map(outcome=>({
    code:"invalid_input",message:"重试原请求核对草稿",requestId:writeRequest.requestId,nativeFailure:{...matchedFailure,kind,outcome}}))),
  {code:"invalid_input",message:"重试原请求核对草稿",nativeFailure:null},
  hbsDocumentError(hbsDocumentError({code:"invalid_input",message:"重试原请求核对草稿",requestId:writeRequest.requestId,
    nativeFailure:{...matchedFailure,extra:"PRIVATE_BODY"}})),
])("retains the exact write dispatch on technical, unknown, damaged or unbound errors (%#)",async reason=>{
  selectAppRuntime({mode:"desktop",storage,gateways:{},dispose(){}});
  vi.mocked(invoke).mockRejectedValueOnce(reason).mockResolvedValueOnce(invalidWrite);
  const error=await write().catch((failure:unknown)=>failure) as Error;
  expect(error).toBeInstanceOf(Error);expect(`${error.message} ${JSON.stringify(error)}`).not.toMatch(/PRIVATE_|\/private/u);
  expect(JSON.parse(storage.getItem(writeKey)!)).toEqual(writeRequest);
  await expect(write("retry-click")).resolves.toEqual(invalidWrite);
  expect(vi.mocked(invoke).mock.calls.map(([,args])=>(args as {draft:unknown}).draft)).toEqual([writeRequest,writeRequest]);
  expect(storage.getItem(writeKey)).toBeNull();
});
