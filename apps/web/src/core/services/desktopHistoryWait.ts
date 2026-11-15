type DesktopInvoke = (command: string, args: Record<string, unknown>) => Promise<unknown>;

/** Only read-only history waits may be cancelled after dispatch. */
export async function invokeDesktopHistoryWait(
  invoke: DesktopInvoke,
  args: Record<string, unknown>,
  signal: AbortSignal,
): Promise<unknown> {
  const abortError = (): unknown => signal.reason ?? new DOMException("Aborted", "AbortError");
  if (signal.aborted) throw abortError();
  const resourceId = await invoke("agent_create_history_wait", {});
  if (!Number.isSafeInteger(resourceId) || Number(resourceId) < 0) throw new Error("agent_history_wait_resource_invalid");

  // Retain a settled promise so an abort-triggered cleanup can never become
  // an unhandled rejection while the wait command is still finishing.
  let closing: Promise<{error?: unknown}> | undefined;
  const close = (): Promise<{error?: unknown}> => closing ??= Promise.resolve()
    .then(() => invoke("agent_cancel_history_wait", {resourceId}))
    .then(() => ({}), error => ({error}));
  const onAbort = (): void => { void close(); };
  signal.addEventListener("abort", onAbort, {once: true});
  let result: unknown;
  let failure: unknown;
  let failed = false;
  try {
    // Includes cancellation while the resource creation IPC was in flight.
    if (signal.aborted) throw abortError();
    result = await invoke("agent_wait_for_history_cancellable", {...args, resourceId});
    if (signal.aborted) throw abortError();
  } catch (error) {
    failed = true;
    failure = error;
  } finally {
    signal.removeEventListener("abort", onAbort);
  }
  const closeResult = await close();
  if (failed) throw failure;
  if ("error" in closeResult) throw closeResult.error;
  return result;
}
