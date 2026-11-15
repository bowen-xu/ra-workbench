/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useAgentQueueAutoPrepare } from "./useAgentQueueAutoPrepare";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("prepares one FIFO head after a running turn becomes idle", async () => {
  const prepare = vi.fn();
  let mark!: () => void;
  const root = createRoot(document.createElement("div"));
  function Probe({ running, blocked }: { running: boolean; blocked: boolean }) {
    ({ markQueued: mark } = useAgentQueueAutoPrepare(running, { inputId: "head", text: "next" }, blocked, prepare));
    return null;
  }
  try {
    await act(async () => root.render(<Probe running blocked={false} />));
    mark();
    await act(async () => root.render(<Probe running={false} blocked={false} />));
    expect(prepare).toHaveBeenCalledOnce();
    expect(prepare).toHaveBeenCalledWith({ inputId: "head", text: "next" });
    await act(async () => root.render(<Probe running={false} blocked={false} />));
    expect(prepare).toHaveBeenCalledOnce();
  } finally {
    await act(async () => root.unmount());
  }
});

it("automatically prepares an existing idle queue head", async () => {
  const prepare = vi.fn();
  const root = createRoot(document.createElement("div"));
  function Probe() {
    useAgentQueueAutoPrepare(false, { inputId: "head", text: "next" }, false, prepare);
    return null;
  }
  try {
    await act(async () => root.render(<Probe />));
    expect(prepare).toHaveBeenCalledOnce();
    expect(prepare).toHaveBeenCalledWith({ inputId: "head", text: "next" });
  } finally {
    await act(async () => root.unmount());
  }
});

it("retains multiple FIFO triggers until each active turn finishes", async () => {
  const prepare = vi.fn();
  let mark!: () => void;
  const root = createRoot(document.createElement("div"));
  function Probe({ running }: { running: boolean }) {
    ({ markQueued: mark } = useAgentQueueAutoPrepare(running, { inputId: "head", text: "next" }, false, prepare));
    return null;
  }
  try {
    await act(async () => root.render(<Probe running />));
    mark(); mark();
    await act(async () => root.render(<Probe running={false} />));
    expect(prepare).toHaveBeenCalledOnce();
    await act(async () => root.render(<Probe running />));
    await act(async () => root.render(<Probe running={false} />));
    expect(prepare).toHaveBeenCalledTimes(2);
  } finally {
    await act(async () => root.unmount());
  }
});

it("keeps the request pending while the host is blocked", async () => {
  const prepare = vi.fn();
  let mark!: () => void;
  const root = createRoot(document.createElement("div"));
  function Probe({ running, blocked }: { running: boolean; blocked: boolean }) {
    ({ markQueued: mark } = useAgentQueueAutoPrepare(running, { inputId: "head", text: "next" }, blocked, prepare));
    return null;
  }
  try {
    await act(async () => root.render(<Probe running blocked />));
    mark();
    await act(async () => root.render(<Probe running={false} blocked />));
    expect(prepare).not.toHaveBeenCalled();
    await act(async () => root.render(<Probe running={false} blocked={false} />));
    expect(prepare).toHaveBeenCalledOnce();
  } finally {
    await act(async () => root.unmount());
  }
});

it("does not latch the head if a live queue reload wins after the render snapshot", async () => {
  const prepare = vi.fn().mockReturnValueOnce(false).mockReturnValue(undefined);
  const root = createRoot(document.createElement("div"));
  const head = {inputId:"command",text:"/compact"};
  function Probe({blocked}: {blocked:boolean}) {
    useAgentQueueAutoPrepare(false, head, blocked, prepare);
    return null;
  }
  try {
    await act(async()=>root.render(<Probe blocked={false}/>));
    expect(prepare).toHaveBeenCalledOnce();
    await act(async()=>root.render(<Probe blocked/>));
    await act(async()=>root.render(<Probe blocked={false}/>));
    expect(prepare).toHaveBeenCalledTimes(2);
    await act(async()=>root.render(<Probe blocked={false}/>));
    expect(prepare).toHaveBeenCalledTimes(2);
  } finally {await act(async()=>root.unmount());}
});
