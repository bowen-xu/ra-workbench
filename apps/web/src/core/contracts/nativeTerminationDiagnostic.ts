/** Safe metadata for an initial signal error after Core confirms cleanup. */
export interface NativeTerminationDiagnostic {
  readonly operation: "group_terminate";
  readonly kind: "PermissionDenied";
  readonly rawOsError: 1;
}
