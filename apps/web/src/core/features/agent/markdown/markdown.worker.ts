import { MarkdownCompiler } from "./compileMarkdown";
import type { MarkdownReply, MarkdownRequest } from "./markdownProtocol";

const scope = self as unknown as {
  onmessage: ((event: MessageEvent<MarkdownRequest>) => void) | null;
  postMessage(reply: MarkdownReply): void;
};
const messages = new Map<number, MarkdownCompiler>();
scope.onmessage = ({data}) => {
  if (data.kind === "release") { messages.delete(data.owner); return; }
  const compiler = messages.get(data.owner) ?? new MarkdownCompiler();
  messages.set(data.owner, compiler);
  try {
    scope.postMessage({kind: "rendered", owner: data.owner, revision: data.revision, blocks: compiler.render(data.input)});
  } catch {
    // Keep diagnostics stable and content-free. The UI retains exact plain text.
    scope.postMessage({kind: "failed", owner: data.owner, revision: data.revision, code: "AGENT_MARKDOWN_RENDER_FAILED"});
  }
};
