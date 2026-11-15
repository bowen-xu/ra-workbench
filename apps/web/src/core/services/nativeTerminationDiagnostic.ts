import type {NativeTerminationDiagnostic} from "../contracts/nativeTerminationDiagnostic";

export function nativeTerminationDiagnostic(reason: unknown): NativeTerminationDiagnostic | undefined {
  if (reason === null || typeof reason !== "object" || Array.isArray(reason)) return undefined;
  const value = reason as Record<string, unknown>;
  if (Reflect.ownKeys(value).length !== 3
    || !Object.hasOwn(value, "operation") || !Object.hasOwn(value, "kind") || !Object.hasOwn(value, "rawOsError")
    || value.operation !== "group_terminate" || value.kind !== "PermissionDenied" || value.rawOsError !== 1) return undefined;
  // Copy fixed literals, never arbitrary child strings, getters or extra data.
  return {operation: "group_terminate", kind: "PermissionDenied", rawOsError: 1};
}

export function nativeTerminationMessage(reason: unknown): string | undefined {
  return nativeTerminationDiagnostic(reason) === undefined ? undefined
    : "终止阶段诊断（已确认完成收尾）：operation=group_terminate，kind=PermissionDenied，rawOsError=1";
}
