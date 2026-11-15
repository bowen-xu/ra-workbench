import type { MarkdownWorkerPort } from "./MarkdownRenderQueue";

export function createMarkdownWorker(): MarkdownWorkerPort {
  return new Worker(new URL("./markdown.worker.ts", import.meta.url), {type: "module"}) as MarkdownWorkerPort;
}
