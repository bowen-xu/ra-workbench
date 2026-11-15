import { vi } from "vitest";
// Only the browser Worker transport is mocked. Tests run the production compiler
// and structured-clone its messages; there is no alternative Markdown algorithm.
vi.mock("./createMarkdownWorker", async () => {
  const {MarkdownCompiler} = await import("./compileMarkdown");
  return {createMarkdownWorker: () => {
    const compilers = new Map<number, InstanceType<typeof MarkdownCompiler>>();
    let terminated = false;
    const port: import("./MarkdownRenderQueue").MarkdownWorkerPort = {
      onmessage: null, onerror: null, onmessageerror: null,
      terminate() {terminated = true; compilers.clear();},
      postMessage(request) {
        Promise.resolve().then(() => {
          if (terminated) return;
          if (request.kind === "release") {compilers.delete(request.owner); return;}
          const compiler = compilers.get(request.owner) ?? new MarkdownCompiler();
          compilers.set(request.owner, compiler);
          const reply = {kind: "rendered" as const, owner: request.owner, revision: request.revision,
            blocks: compiler.render(structuredClone(request.input))};
          port.onmessage?.({data: structuredClone(reply)} as MessageEvent<typeof reply>);
        });
      },
    };
    return port;
  }};
});
