import { doclingErrorMessage } from "../../services/doclingError";
import { useEffect, useRef, useState } from "react";
import type {
  DoclingGateway,
  DoclingSettingsDraft,
} from "../../services/doclingGateway";
import { notify } from "../../services/notifications";

const DEFAULT_DRAFT: DoclingSettingsDraft = {
  enabled: true,
  modelManifestUrl: "http://127.0.0.1:4174/models/docling/latest.json",
  defaultPort: 5001,
  fallbackStartPort: 5010,
  fallbackEndPort: 5099,
  idleShutdownMs: 1_200_000,
  requestTimeoutMs: 300_000,
};

/** Serialize writes and flush the latest edit before using the configuration. */
export function useDoclingSettings(gateway: DoclingGateway, enabled = true) {
  const [draft, setDraft] = useState(DEFAULT_DRAFT);
  const [loaded, setLoaded] = useState(false);
  const latest = useRef(DEFAULT_DRAFT);
  const pending = useRef<DoclingSettingsDraft | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const writes = useRef<Promise<void>>(Promise.resolve());
  const loadedGateway = useRef<DoclingGateway | undefined>(undefined);

  function flush(): Promise<void> {
    clearTimeout(timer.current);
    const value = pending.current;
    if (!value) return writes.current;
    pending.current = undefined;
    const operation = writes.current
      .catch(() => undefined)
      .then(async () => {
        try {
          if (value.fallbackStartPort > value.fallbackEndPort) {
            throw new Error("fallback 起始端口不能大于结束端口");
          }
          await gateway.saveSettings(value);
        } catch (reason) {
          if (!pending.current && latest.current === value)
            pending.current = value;
          notify.error(`Docling 设置保存失败：${doclingErrorMessage(reason)}`);
          throw reason;
        }
      });
    writes.current = operation;
    return operation;
  }

  function update(patch: Partial<DoclingSettingsDraft>): void {
    latest.current = { ...latest.current, ...patch };
    setDraft(latest.current);
    pending.current = latest.current;
    clearTimeout(timer.current);
    timer.current = setTimeout(() => void flush().catch(() => undefined), 400);
  }

  useEffect(() => {
    if (!enabled || loadedGateway.current === gateway) return;
    let active = true;
    void gateway
      .getSettings()
      .then((settings) => {
        if (!active) return;
        latest.current = settings;
        loadedGateway.current = gateway;
        setDraft(settings);
        setLoaded(true);
      })
      .catch((reason) => notify.error(doclingErrorMessage(reason)));
    return () => {
      active = false;
    };
    // Each mounted settings page owns one stable gateway and save queue.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gateway, enabled]);

  // Visibility pauses reads only. The existing save queue keeps its timer and
  // still flushes on disposal, including edits made just before switching.
  useEffect(() => () => {void flush().catch(() => undefined);}, [gateway]);

  return { draft, loaded, update, flush };
}
