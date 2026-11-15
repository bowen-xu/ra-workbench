import { ControlHint } from "../../components/ControlHint";
import { useEffect, useRef, useState } from "react";
import { GitBranch } from "lucide-react";
import type { AgentConversationBinding, AgentConversationSummary, AgentForkOrigin } from "@research-assistant/core/agent";
import type { AgentForkOriginGateway } from "@research-assistant/data/agent";
import { agentForkOriginGateway } from "../../services/agentGateway";
import { diagnosticError } from "../../services/diagnosticError";
import { AgentTransientFailure } from "./AgentFailureMessage";
import "./ConversationForkControl.css";

interface Props {
  readonly conversationId: string;
  readonly binding: AgentConversationBinding;
  readonly onOpen: (parent: AgentConversationSummary, throughTurnId: string) => void;
  readonly gateway?: AgentForkOriginGateway;
}
/** Key by conversation/host. This view reads lineage only; navigation requires an explicit click. */
export function ConversationForkOrigin({ conversationId, binding, onOpen, gateway = agentForkOriginGateway }: Props): React.JSX.Element | null {
  const [state, setState] = useState<{ loading: boolean; error: string | null; origin: AgentForkOrigin | null }>({ loading: true, error: null, origin: null });
  const generation = useRef(0);
  async function load(): Promise<void> {
    const request = ++generation.current;
    setState({ loading: true, error: null, origin: null });
    try {
      const result = await gateway.read({ conversationId, binding });
      if (request === generation.current) setState({ loading: false, error: null, origin: result.origin });
    } catch (reason) {
      if (request === generation.current) {
        const safe = diagnosticError(reason);
        const fields = [`code=${typeof safe.code === "string" ? safe.code : "FORK_ORIGIN_READ_FAILED"}`];
        if (/^[\w:.-]{1,200}$/.test(conversationId)) fields.push(`conversation=${conversationId}`);
        if (typeof safe.requestId === "string") fields.push(`request=${safe.requestId}`);
        if (typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599) fields.push(`HTTP ${safe.httpStatus}`);
        if (typeof safe.cause === "string") fields.push(`cause=${safe.cause}`);
        setState({ loading: false, error: fields.join("；"), origin: null });
      }
    }
  }
  useEffect(() => { void load(); return () => { generation.current += 1; }; }, []);
  if (state.loading) return null;
  if (state.error) return <AgentTransientFailure id="fork-origin" content="无法读取分支来源，请重试。" diagnosticText={state.error} retry={() => void load()} />;
  const origin = state.origin;
  if (!origin) return null;
  return <div className="conversation-fork-control conversation-fork-origin" aria-label="分支来源">
    <GitBranch aria-hidden="true" size={16} />
    <span>继续自“{origin.parent.title}”{origin.parent.archived ? "（已归档）" : ""}</span>
    <ControlHint content="打开父会话并定位到创建此分支的轮次，不发送消息"><button type="button"
      onClick={() => onOpen(origin.parent, origin.throughTurnId)}>查看来源轮次</button></ControlHint>
  </div>;
}
