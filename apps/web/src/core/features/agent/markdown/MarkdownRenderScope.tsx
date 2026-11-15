import { createContext, useContext, useState, type ReactNode } from "react";
import { MarkdownRenderQueue } from "./MarkdownRenderQueue";
import { createMarkdownWorker } from "./createMarkdownWorker";

const context = createContext<MarkdownRenderQueue | null>(null);
function createQueue(): MarkdownRenderQueue {
  return new MarkdownRenderQueue(createMarkdownWorker);
}
/** The transcript owns a single lazy worker, released when its last message
 * unmounts. A standalone Markdown surface owns its own queue, never a global. */
export function MarkdownRenderScope({children}: {readonly children: ReactNode}): React.JSX.Element {
  const [queue] = useState(createQueue);
  return <context.Provider value={queue}>{children}</context.Provider>;
}
export function useMarkdownRenderQueue(): MarkdownRenderQueue {
  const shared = useContext(context);
  const [local] = useState(createQueue);
  return shared ?? local;
}
