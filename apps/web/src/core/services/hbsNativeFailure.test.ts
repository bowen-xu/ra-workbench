import {expect,it} from "vitest";
import {hbsDocumentError,hbsNativeDiagnostic,hbsNativeRejectionIsDefinite} from "./hbsNativeFailure";

const failure={kind:"pureRejected",phase:"validation",outcome:"rolledBack",pluginId:"literature_library",code:"invalid_input",
  packageKey:"a".repeat(64),instanceId:"actual-instance",nativeRequestId:"actual-native-request"};

it("does not invent missing Core or Native request identities",()=>{
  const error=hbsDocumentError({code:"MODEL_INVALID_DOCUMENT",message:"原错误",nativeFailure:{kind:"technical",phase:"sources",outcome:"notStarted",
    pluginId:"literature_library",code:"plugin_package_missing"}}) as Error;
  expect(error.message).toContain("plugin_package_missing");expect(error.message).not.toMatch(/requestId=|nativeRequestId=|undefined|null/u);
  expect(error).not.toHaveProperty("requestId");expect(hbsNativeRejectionIsDefinite(error,"original-core")).toBe(false);
});

it("strips unsafe identity, integer, cause and termination values from both message and export",()=>{
  const error=hbsDocumentError({code:"MODEL_STORAGE_ERROR",message:"原安全错误",requestId:"original-core",nativeFailure:{...failure,kind:"technical",
    packageKey:"../private/key",instanceId:"bad instance",nativeRequestId:"/private/request",operationId:{body:"PRIVATE_BODY"},exitCode:2**40,signal:0.5,
    dataError:{code:"plugin_data_sql_failed",phase:"read",resourceId:"/private/resource",resourceVersion:0,safeCause:"SELECT body FROM private",
      sqliteCode:{body:"PRIVATE_BODY"},sqliteExtendedCode:2**40,sql:"PRIVATE_SQL"},
    terminationError:{operation:"group_terminate",kind:"PermissionDenied",rawOsError:1,body:"PRIVATE_BODY"},
    cleanupError:"/private/input",message:"PRIVATE_MESSAGE",headers:{authorization:"PRIVATE_HEADER"}}}) as Error;
  expect(error.message).toContain("requestId=original-core");
  expect(`${error.message} ${JSON.stringify(error)} ${JSON.stringify(hbsNativeDiagnostic(error))}`).not.toMatch(/PRIVATE_|private|bad instance|SELECT/u);
  expect(hbsNativeRejectionIsDefinite(error,"original-core")).toBe(false);
});

it("projects database diagnostics through the shared Data whitelist without resource fields",()=>{
  const error=hbsDocumentError({code:"MODEL_STORAGE_ERROR",message:"读取失败",requestId:"original-core",nativeFailure:{...failure,kind:"technical",
    phase:"rollback",outcome:"unknown",databaseError:{code:"storage_error",phase:"commit",safeCause:"sqlite_operation_failed",sqliteCode:"ConstraintViolation",sqliteExtendedCode:787,
      packageKey:"a".repeat(64),resourceId:"private-resource",resourceVersion:1,originalSqlFailure:"PRIVATE_SQL"},
    rollbackError:{code:"storage_error",phase:"rollback",safeCause:"sqlite_operation_failed"}}}) as Error;
  expect(hbsNativeDiagnostic(error)).toMatchObject({nativeFailure:{databaseError:{code:"storage_error",phase:"commit",safeCause:"sqlite_operation_failed",
    sqliteCode:"ConstraintViolation",sqliteExtendedCode:787},rollbackError:{code:"storage_error",phase:"rollback",safeCause:"sqlite_operation_failed"}}});
  expect(error.message).not.toMatch(/resourceId|resourceVersion|private-resource|PRIVATE/u);
});

it.each([undefined,null,[],{}, {...failure,kind:"future"},{...failure,pluginId:"other-plugin"}, {...failure,code:"unsafe code"}])(
  "keeps malformed Native blocks distinct from the original static path (%#)",nativeFailure=>{
    const error=hbsDocumentError({code:"MODEL_INVALID_DOCUMENT",message:"原错误",requestId:"original-core",nativeFailure});
    expect(hbsNativeRejectionIsDefinite(error,"original-core")).toBe(false);
    expect(hbsNativeDiagnostic(error)).toEqual({});
  });

it("leaves original static Error identity and unrelated thrown values unchanged",()=>{
  const error=Object.assign(new Error("original\nstatic message"),{code:"MODEL_INVALID_DOCUMENT",requestId:"original-core"});
  expect(hbsDocumentError(error)).toBe(error);expect(hbsNativeRejectionIsDefinite(error,"original-core")).toBeUndefined();
  expect(hbsDocumentError("connection lost")).toBe("connection lost");
});

it("does not turn a damaged pure marker into a definite rejection when an error is adapted again",()=>{
  const error=hbsDocumentError({code:"MODEL_INVALID_DOCUMENT",message:"原错误",requestId:"original-core",nativeFailure:{...failure,body:"PRIVATE_BODY"}});
  expect(hbsNativeRejectionIsDefinite(error,"original-core")).toBe(false);
  const adapted=hbsDocumentError(error);
  expect(hbsNativeRejectionIsDefinite(adapted,"original-core")).toBe(false);
  expect(JSON.stringify(adapted)).not.toContain("PRIVATE_BODY");
});
