import {describe, expect, it} from "vitest";
import {aiAssistanceErrorMessage} from "./aiAssistanceErrorMessage";
const packageKey = "a".repeat(64);
const instance = "10000000-0000-4000-8000-000000000001";
const request = "10000000-0000-4000-8000-000000000002";

it("keeps finite analysis history identity and safe source diagnostics", () => {
  const text = aiAssistanceErrorMessage({analysisHistoryOperation:"get", code:"database_error", message:"原读取失败",
    workflowId:"historical-workflow", paperId:"paper", rawSql:"PRIVATE_SQL",
    nativeFailure:{kind:"technical",phase:"source-read",outcome:"notStarted",pluginId:"literature_library",
      code:"plugin_data_database_failed",packageKey,instanceId:instance,nativeRequestId:request,
      databaseError:{phase:"source-read",code:"database_error",safeCause:"invalid_column_type",columnIndex:4,rawPayload:"PRIVATE_BODY"}}});
  for (const field of ["historical-workflow","paperId=paper",packageKey,instance,request,"invalid_column_type"]) expect(text).toContain(field);
  expect(text).not.toMatch(/PRIVATE|rawSql|rawPayload/u);
});

it("keeps unknown analysis operations on the original error formatter", () => {
  expect(aiAssistanceErrorMessage({analysisHistoryOperation:"unbounded-query",code:"invalid_input",message:"原错误",workflowId:"PRIVATE"})).toBe("原错误（错误码：invalid_input）");
});
describe("finite paper preparation errors", () => {
  it("retains available Native identity and screenshot progress without a conversation identity", () => {
    const text = aiAssistanceErrorMessage({paperContextOperation:"prepare",code:"plugin_host_exited",message:"文献宿主已退出。",pluginId:"literature_library",packageKey,instanceId:instance,requestId:request,exitCode:42,imageObjectWriteStarted:true,savedImageAssetCount:1,rawStderr:"PRIVATE_STDERR",database:"PRIVATE_DATABASE"});
    for (const field of [packageKey,instance,request,"exitCode=42","savedImageAssetCount=1","imageObjectWriteStarted=true"]) expect(text).toContain(field);
    expect(text).not.toMatch(/PRIVATE|conversationId|pageId|manuscriptId/u);
  });
  it("reuses the existing finite HBS diagnostic projection for domain and data failures", () => {
    const text = aiAssistanceErrorMessage({paperContextOperation:"preview",code:"corrupt",message:"论文快照校验失败。",nativeFailure:{kind:"sourceData",phase:"source-read",outcome:"notStarted",pluginId:"literature_library",code:"plugin_data_sql_failed",packageKey,instanceId:instance,nativeRequestId:request,dataError:{code:"plugin_data_sql_failed",resourceId:"literature.paper-context.session-summary",phase:"read",sqliteCode:"AuthorizationForStatementDenied",sqliteExtendedCode:23,rawSql:"PRIVATE_SQL"},rawPayload:"PRIVATE_BODY"}});
    for (const field of [packageKey,instance,request,"code=corrupt","AuthorizationForStatementDenied","sqliteExtendedCode=23"]) expect(text).toContain(field);
    expect(text).not.toMatch(/PRIVATE|rawPayload|rawSql/u);
  });
  it("keeps old errors on their original formatter", () => {
    expect(aiAssistanceErrorMessage({code:"invalid_input",message:"原错误",packageKey,requestId:request})).toBe("原错误（错误码：invalid_input）");
  });
  it("does not add absent identities or damaged image counters", () => {
    const text = aiAssistanceErrorMessage({paperContextOperation:"prepare",code:"invalidInput",message:"截图格式无效。",savedImageAssetCount:-1,imageObjectWriteStarted:"yes",packageKey:"PRIVATE_PATH/secret",requestId:"PRIVATE_PATH/secret"});
    expect(text).toBe("截图格式无效。 · code=invalidInput");
  });
});

it("preserves finite retrieval identities, provider cause and actual cleanup progress", () => {
  const text = aiAssistanceErrorMessage({retrievalOperation:"preview",code:"provider_unavailable",message:"查询 embedding provider 暂时不可用",
    clientRequestId:request,traceId:instance,leaseReleaseConfirmed:true,traceWriteStarted:false,
    providerError:{code:"rate_limited",retryAfterMs:5000,response:"PRIVATE_RESPONSE"},authorization:"PRIVATE_KEY"});
  for (const field of [`clientRequestId=${request}`,`traceId=${instance}`,"providerError.code=rate_limited","providerError.retryAfterMs=5000","leaseReleaseConfirmed=true","traceWriteStarted=false"]) expect(text).toContain(field);
  expect(text).not.toMatch(/PRIVATE|nativeRequestId|packageKey/u);
});

it("keeps retrieval SQL diagnostics and filters damaged supplemental fields", () => {
  const text = aiAssistanceErrorMessage({retrievalOperation:"trace-read",code:"storage_error",message:"检索读取失败。",traceId:instance,
    leaseReleaseConfirmed:"yes",traceWriteStarted:null,providerError:{code:"PRIVATE_PATH/response",retryAfterMs:-1},
    nativeFailure:{kind:"sourceData",phase:"read",outcome:"notStarted",pluginId:"literature_library",code:"plugin_data_sql_failed",dataError:{code:"plugin_data_sql_failed",resourceId:"literature.concept-retrieval.trace-seeds",phase:"read",sqliteCode:"TypeMismatch",sqliteExtendedCode:20,rawSql:"PRIVATE_SQL"}}});
  for (const field of ["TypeMismatch","sqliteExtendedCode=20",`traceId=${instance}`]) expect(text).toContain(field);
  expect(text).not.toMatch(/PRIVATE|leaseReleaseConfirmed|traceWriteStarted|retryAfterMs/u);
});

it("retains the actual restored-turn scope beside Native identity", () => {
  const turn = "10000000-0000-4000-8000-000000000003";
  const paper = "10000000-0000-4000-8000-000000000004";
  const text = aiAssistanceErrorMessage({
    paperContextOperation: "preview",
    code: "plugin_host_exited",
    message: "文献宿主已退出。",
    packageKey,
    instanceId: instance,
    requestId: request,
    conversationId: instance,
    turnId: turn,
    paperId: paper,
    path: "PRIVATE_PATH",
  });
  for (const field of [`conversationId=${instance}`, `turnId=${turn}`, `paperId=${paper}`, `nativeRequestId=${request}`]) {
    expect(text).toContain(field);
  }
  expect(text).not.toContain("PRIVATE_PATH");
});

it("distinguishes submission request identity from the actual Native request and preserves SQL settlement", () => {
  const coreRequest = "10000000-0000-4000-8000-000000000003";
  const snapshot = "10000000-0000-4000-8000-000000000004";
  const text = aiAssistanceErrorMessage({paperContextOperation: "submission", code: "storage",
    message: "论文提交来源读取失败。", conversationId: instance, requestId: coreRequest, paperId: instance,
    paperContextSnapshotId: snapshot,
    reason: {code: "DATA_SQL_CORRUPT", reason: {operationId: coreRequest, epoch: snapshot, sqlFence: "42",
      phase: "source-read", sqliteCode: "DatabaseCorrupt", sqliteExtendedCode: 11, safeCause: "paper_prepare_corrupt"}},
    nativeFailure: {kind: "sourceData", phase: "source-read", outcome: "notStarted", pluginId: "literature_library",
      code: "plugin_data_sql_failed", packageKey, instanceId: instance, nativeRequestId: request},
    privateBody: "PRIVATE_BODY"});
  for (const field of ["DATA_SQL_CORRUPT", "sqlFence=42", "DatabaseCorrupt", `requestId=${coreRequest}`,
    `nativeRequestId=${request}`, `paperContextSnapshotId=${snapshot}`, packageKey]) expect(text).toContain(field);
  expect(text).not.toContain("PRIVATE_BODY");
});

it("retains the scoped index request and finite SQL cause through the shared formatter", () => {
  const text = aiAssistanceErrorMessage({indexOperation:"provider-backfill",code:"storage_error",message:"语义索引控制状态保存失败。",
    representationId:instance,indexCause:{code:"storage_error",phase:"write",safeCause:"sqlite_operation_failed",sqliteCode:"ConstraintViolation",sqliteExtendedCode:1811,rawSql:"PRIVATE_SQL",message:"PRIVATE_BODY"},
    database:"PRIVATE_DATABASE",authorization:"PRIVATE_KEY"});
  for (const field of [`representationId=${instance}`,"indexCause.sqliteExtendedCode=1811","indexCause.safeCause=sqlite_operation_failed","ConstraintViolation"]) expect(text).toContain(field);
  expect(text).not.toMatch(/PRIVATE|nativeRequestId|packageKey/u);
});

it("keeps actual index Native identity and rollback outcome without inventing job identity", () => {
  const text = aiAssistanceErrorMessage({indexOperation:"retry",code:"plugin_host_response_mismatch",message:"索引结果与原输入不匹配。",representationId:instance,
    nativeFailure:{kind:"technical",phase:"validation",outcome:"rolledBack",pluginId:"literature_library",code:"plugin_host_response_mismatch",packageKey,instanceId:instance,nativeRequestId:request,rawPayload:"PRIVATE_BODY"}});
  for (const field of [packageKey,instance,request,"rolledBack",`representationId=${instance}`]) expect(text).toContain(field);
  expect(text).not.toMatch(/PRIVATE|jobId=|embeddingSpaceId=/u);
});

it("limits index supplemental projection to the existing finite operation names", () => {
  const text = aiAssistanceErrorMessage({indexOperation:"retry.execute-sql",code:"storage_error",message:"索引请求失败。",jobId:instance,indexCause:{sqliteCode:"PRIVATE_METADATA"}});
  expect(text).toBe("索引请求失败。（错误码：storage_error）");
});
