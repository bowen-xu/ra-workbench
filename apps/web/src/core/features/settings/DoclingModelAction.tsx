import { doclingErrorMessage } from "../../services/doclingError";
import { Check, Download, LoaderCircle, RefreshCw } from "lucide-react";
import { useState } from "react";
import { ControlHint } from "../../components/ControlHint";
import type {
  DoclingGateway,
  DoclingModelStatus,
} from "../../services/doclingGateway";
import { notify } from "../../services/notifications";

export function DoclingModelAction({
  gateway,
  models,
  setModels,
  disabled,
  flush,
}: {
  readonly gateway: DoclingGateway;
  readonly models: DoclingModelStatus | undefined;
  readonly setModels: (value: DoclingModelStatus) => void;
  readonly disabled: boolean;
  readonly flush: () => Promise<void>;
}): React.JSX.Element {
  const [phase, setPhase] = useState<
    "check" | "checking" | "download" | "downloading" | "done"
  >("check");
  const downloading =
    phase === "downloading" ||
    models?.state === "downloading" ||
    models?.state === "verifying";
  const working = downloading || phase === "checking";
  const label = downloading
    ? models?.state === "verifying"
      ? "正在校验模型"
      : "正在下载模型"
    : phase === "checking"
      ? "正在检查模型更新"
      : phase === "download"
        ? "下载模型更新"
        : phase === "done"
          ? "模型更新完成"
          : "检查模型更新";
  const Icon = working
    ? LoaderCircle
    : phase === "download"
      ? Download
      : phase === "done"
        ? Check
        : RefreshCw;

  async function run(): Promise<void> {
    const download = phase === "download";
    setPhase(download ? "downloading" : "checking");
    try {
      await flush();
      const value = await (download
        ? gateway.downloadModels()
        : gateway.checkModelUpdate());
      setModels(value);
      if (download) {
        setPhase("done");
        notify.success("FP32 模型更新完成");
      } else {
        const available =
          value.availableIdentity != null
            ? value.availableIdentity !== value.installedIdentity
            : value.availableVersion != null &&
              value.availableVersion !== value.installedVersion;
        setPhase(available ? "download" : "check");
        if (available) notify.success("有模型更新，可点击下载");
        else notify.success("模型无更新");
      }
    } catch (reason) {
      setPhase(download ? "download" : "check");
      notify.error(doclingErrorMessage(reason));
      await gateway
        .getModelStatus()
        .then(setModels)
        .catch(() => undefined);
    }
  }

  return (
    <ControlHint content={label} wrap>
      <button
        type="button"
        className="settings-icon-button"
        aria-label={label}
        aria-busy={working}
        disabled={disabled || working}
        onMouseEnter={() => {
          if (phase === "done") setPhase("check");
        }}
        onClick={() => void run()}
      >
        <Icon
          size={16}
          aria-hidden="true"
          className={working ? "settings-action-spinning" : undefined}
        />
      </button>
    </ControlHint>
  );
}
