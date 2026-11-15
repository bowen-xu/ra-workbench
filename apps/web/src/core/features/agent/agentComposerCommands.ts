/** UI command identities are independent of resource references and model input. */
export const agentComposerCommands = [
  { name: "compact", description: "压缩当前对话上下文" },
] as const;

/** A command occupies the whole input. Paths and slashes in prose stay ordinary text. */
export function readComposerCommand(text: string): { name: string; arguments: string } | null {
  const match = /^[ \t]*\/([a-zA-Z][\w-]*|)(?:[ \t]+([^\r\n]*))?[ \t]*$/u.exec(text);
  return match ? { name: match[1]!, arguments: (match[2] ?? "").trim() } : null;
}

export function composerCommandQuery(text: string, caret: number): string | null {
  if (!/^[ \t]*\/[\w-]*[ \t]*$/u.test(text)) return null;
  const match = /^[ \t]*\/([\w-]*)$/u.exec(text.slice(0, caret));
  return match?.[1] ?? null;
}
