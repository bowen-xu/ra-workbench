import {Component, type ReactNode} from "react";
import type {PluginViewProps} from "@research-assistant/workbench-sdk/activation";
import type {RegisteredPluginView} from "../../../core/runtime/plugins/viewRegistry";
import {diagnosticError} from "../../../core/public/diagnostics";

export interface RuntimePluginViewFailure {
  readonly code: "plugin_view_render_failed" | "plugin_view_recovery_failed";
  readonly pluginId: string;
  readonly viewId: string;
  readonly cause: string;
  readonly sourceCode?: string;
  readonly requestId?: string;
  readonly turnId?: string;
  readonly operationId?: string;
  readonly taskId?: string;
  readonly httpStatus?: number;
}

interface Props {
  readonly registration: RegisteredPluginView;
  readonly viewProps: PluginViewProps;
  /** A finite role of this exact registration can reuse the same failure UI. */
  readonly renderContent?: (props: PluginViewProps) => ReactNode;
  readonly onFailure: (registration: RegisteredPluginView, failure: RuntimePluginViewFailure) => void;
  /** App preserves and opens this exact registration; rejection keeps the failure visible. */
  readonly onRecover: (registration: RegisteredPluginView) => void | Promise<void>;
}
interface State {
  readonly registration: RegisteredPluginView;
  readonly registrationGeneration: number;
  readonly failed: boolean;
  readonly reason: unknown;
  readonly recoveryReason: unknown;
  readonly recoveryFailed: boolean;
  readonly recovering: boolean;
}

function failure(registration: RegisteredPluginView, reason: unknown,
  code: RuntimePluginViewFailure["code"]): RuntimePluginViewFailure {
  let safe: Record<string, unknown>;
  try { safe = diagnosticError(reason); }
  catch {
    // A thrown object's own getters/string conversion cannot break the fallback.
    safe = {cause: "unclassified"};
  }
  return {code, pluginId: registration.pluginId, viewId: registration.id,
    cause: typeof safe.cause === "string" ? safe.cause : "unclassified",
    ...(typeof safe.code === "string" ? {sourceCode: safe.code}
      : Array.isArray(safe.codes) && typeof safe.codes[0] === "string" ? {sourceCode: safe.codes[0]} : {}),
    ...(typeof safe.requestId === "string" ? {requestId: safe.requestId} : {}),
    ...(typeof safe.turnId === "string" ? {turnId: safe.turnId} : {}),
    ...(typeof safe.operationId === "string" ? {operationId: safe.operationId} : {}),
    ...(typeof safe.taskId === "string" ? {taskId: safe.taskId} : {}),
    ...(typeof safe.httpStatus === "number" ? {httpStatus: safe.httpStatus} : {})};
}

function diagnosticText(value: RuntimePluginViewFailure): string {
  return [value.code, "pluginId=" + value.pluginId, "viewId=" + value.viewId,
    "cause=" + value.cause, ...(value.sourceCode ? ["code=" + value.sourceCode] : []),
    ...(value.requestId ? ["requestId=" + value.requestId] : []),
    ...(value.turnId ? ["turnId=" + value.turnId] : []),
    ...(value.operationId ? ["operationId=" + value.operationId] : []),
    ...(value.taskId ? ["taskId=" + value.taskId] : []),
    ...(value.httpStatus === undefined ? [] : ["HTTP=" + value.httpStatus])].join(" · ");
}

/** Calling render in a child lets React catch both the contribution function
 * and its descendants, including hooks called directly by that function. */
function RegisteredView({registration, viewProps, renderContent}: Pick<Props, "registration" | "viewProps" | "renderContent">): ReactNode {
  return renderContent ? renderContent(viewProps) : registration.render(viewProps);
}

/** Only the contributed view belongs inside this boundary. Core document,
 * navigation, Agent and storage owners remain mounted by App outside it.
 * Event/Promise failures still belong to their individual operation owners. */
export class RuntimePluginViewBoundary extends Component<Props, State> {
  private mounted = false;
  private recoveryAttempt = 0;
  private recoveringRegistration: RegisteredPluginView | undefined;

  constructor(props: Props) {
    super(props);
    this.state = {registration: props.registration, registrationGeneration: 0, failed: false, reason: undefined,
      recoveryReason: undefined, recoveryFailed: false, recovering: false};
  }

  static getDerivedStateFromProps(props: Props, state: State): State | null {
    return props.registration === state.registration ? null
      : {registration: props.registration, registrationGeneration: state.registrationGeneration + 1,
        failed: false, reason: undefined,
        recoveryReason: undefined, recoveryFailed: false, recovering: false};
  }

  static getDerivedStateFromError(reason: unknown): Partial<State> {
    return {failed: true, reason, recovering: false};
  }

  override componentDidMount(): void { this.mounted = true; }

  override componentDidUpdate(previous: Props): void {
    if (previous.registration !== this.props.registration) {
      this.recoveryAttempt += 1;
      this.recoveringRegistration = undefined;
    }
  }

  override componentWillUnmount(): void {
    this.mounted = false;
    this.recoveryAttempt += 1;
    this.recoveringRegistration = undefined;
  }

  override componentDidCatch(reason: unknown): void {
    const {registration, onFailure} = this.props;
    onFailure(registration, failure(registration, reason, "plugin_view_render_failed"));
  }

  private async recover(registration: RegisteredPluginView): Promise<void> {
    if (!this.mounted || this.props.registration !== registration || !this.state.failed
      || this.recoveringRegistration === registration) return;
    const attempt = ++this.recoveryAttempt;
    this.recoveringRegistration = registration;
    this.setState({recovering: true, recoveryFailed: false, recoveryReason: undefined});
    const current = (): boolean => this.mounted && this.props.registration === registration
      && this.recoveryAttempt === attempt;
    try {
      await this.props.onRecover(registration);
      if (!current()) return;
      this.recoveringRegistration = undefined;
      this.setState({failed: false, reason: undefined, recovering: false,
        recoveryFailed: false, recoveryReason: undefined});
    } catch (reason) {
      if (!current()) return;
      this.recoveringRegistration = undefined;
      this.setState({recovering: false, recoveryFailed: true, recoveryReason: reason});
    }
  }

  override render(): ReactNode {
    const {registration, viewProps} = this.props;
    if (!this.state.failed) return <RegisteredView key={this.state.registrationGeneration}
      registration={registration} viewProps={viewProps} {...(this.props.renderContent ? {renderContent: this.props.renderContent} : {})}/>;
    const issue = failure(registration, this.state.reason, "plugin_view_render_failed");
    return <section className="inline-error" role="alert" aria-label="管理视图无法显示"
      style={{fontSize: "var(--ui-font-body)"}}>
      <p>此管理视图暂时无法显示。</p>
      <p>{diagnosticText(issue)}</p>
      {this.state.recoveryFailed && <p>{diagnosticText(failure(registration,
        this.state.recoveryReason, "plugin_view_recovery_failed"))}</p>}
      <button className="ghost-button" style={{fontSize: "var(--ui-font-label)"}} type="button"
        disabled={this.state.recovering} onClick={() => { void this.recover(registration); }}>
        {this.state.recovering ? "正在重新打开…" : "重新打开"}
      </button>
    </section>;
  }
}
