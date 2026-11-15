import { hasPlatformCapabilities } from "../../runtime/appRuntime";
import { ControlHint } from "../../public/workbench";
import { useEffect, useRef } from "react";
import { Mic } from "lucide-react";

import { notify } from "../../public/feedback";
import type { SpeechInputState } from "./speechRecognition";
import { useSpeechInput } from "./useSpeechInput";
import { useSpeechInputAdapter } from "./SpeechInputAdapter";
import "./PaperAiReadingSpeechInput.css";

interface PaperAiReadingSpeechInputProps {
  readonly disabled: boolean;
  readonly onChange: (value: string) => void;
  readonly onStateChange?: (state: SpeechInputState) => void;
  readonly value: string;
  readonly tooltipClassName?: string;
}

const DISCLOSURE = "语音识别由当前系统的 Web Speech 服务提供，可能使用联网识别。继续将请求麦克风和语音识别权限。";
const UNBUNDLED_DEVELOPMENT_MESSAGE = "开发模式的裸进程不具备 macOS 语音权限声明；请使用 pnpm bundle:dev 生成的临时应用测试语音输入";

export function speechInputHostFailure({
  desktop = hasPlatformCapabilities(),
  hostname = typeof window === "undefined" ? "" : window.location.hostname,
  platform = typeof navigator === "undefined" ? "" : navigator.platform,
  protocol = typeof window === "undefined" ? "" : window.location.protocol,
  tauriInternals = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window,
}: {
  readonly desktop?: boolean;
  readonly hostname?: string;
  readonly platform?: string;
  readonly protocol?: string;
  readonly tauriInternals?: boolean;
} = {}): string | undefined {
  const unbundledDevelopment = tauriInternals
    && /Mac/i.test(platform)
    && (protocol === "http:" || protocol === "https:")
    && (hostname === "127.0.0.1" || hostname === "localhost");
  return desktop && unbundledDevelopment ? UNBUNDLED_DEVELOPMENT_MESSAGE : undefined;
}

export function speechInputStateLabel(state: SpeechInputState): string {
  if (state === "starting") return "正在请求语音输入";
  if (state === "listening") return "正在听写";
  if (state === "stopping") return "正在结束听写";
  return "";
}

export function PaperAiReadingSpeechInput({
  disabled,
  onChange,
  onStateChange,
  value,
  tooltipClassName,
}: PaperAiReadingSpeechInputProps): React.JSX.Element {
  const hasAcceptedDisclosureRef = useRef(false);
  const adapter = useSpeechInputAdapter();
  const hostFailure = adapter ? undefined : speechInputHostFailure();
  const speech = useSpeechInput({
    disabled: disabled || hostFailure !== undefined,
    onChange,
    onFailure: (failure) => {
      if (failure.level === "warning") notify.warning(failure.message);
      else notify.error(failure.message);
    },
    value,
  });

  useEffect(() => onStateChange?.(speech.state), [onStateChange, speech.state]);

  const isActive = speech.state !== "idle";
  const label = hostFailure ?? (!speech.isSupported
    ? "当前环境不支持语音输入"
    : isActive ? "停止输入" : "语音输入");

  function toggle(): void {
    if (hostFailure !== undefined) {
      notify.warning(hostFailure);
      return;
    }
    if (!isActive && !hasAcceptedDisclosureRef.current) {
      const disclosure = adapter ? adapter.disclosure : DISCLOSURE;
      if (disclosure && !window.confirm(disclosure)) return;
      hasAcceptedDisclosureRef.current = true;
    }
    speech.toggle();
  }

  return <ControlHint content={label} {...(tooltipClassName ? {tooltipClassName} : {})}><button
    aria-label={label}
    aria-pressed={isActive}
    className="paper-ai-reading-voice"
    disabled={disabled || !speech.isSupported || speech.state === "stopping"}
    onClick={toggle}

    type="button"
  >
    <Mic aria-hidden="true" />
  </button></ControlHint>;
}
