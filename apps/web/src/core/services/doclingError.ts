/** Only documented IPC fields enter user feedback; never stringify an error object. */
export function doclingErrorMessage(reason: unknown): string {
  if (typeof reason === "string" && reason.trim()) return reason;
  if (typeof reason === "object" && reason !== null) {
    const message =
      "message" in reason && typeof reason.message === "string" && reason.message.trim()
        ? reason.message
        : "Docling 操作失败，请检查本地服务状态后重试";
    const code =
      "code" in reason && typeof reason.code === "string" && /^[a-z][a-z0-9_]{0,63}$/.test(reason.code)
        ? `[${reason.code}] `
        : "";
    return `${code}${message}`;
  }
  return "Docling 操作失败，请检查本地服务状态后重试";
}
