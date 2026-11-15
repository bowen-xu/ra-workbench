import { afterEach, describe, expect, it, vi } from "vitest";
const sonner = vi.hoisted(() => Object.assign(vi.fn(), {
  error: vi.fn(), info: vi.fn(), success: vi.fn(), warning: vi.fn(), dismiss: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: sonner }));
import { getHiddenNotifications, notify } from "./notifications";

afterEach(() => { for (const id of ["task", "result", "review"]) notify.dismiss(id); vi.clearAllMocks(); });
describe("application notifications", () => {
  it("retains product durations and scoped dismissal", () => {
    expect(notify.error("failed", { id: "result" })).toBe("result");
    expect(sonner.error).toHaveBeenLastCalledWith("failed", expect.objectContaining({ duration: 7_000 }));
    notify.warning("review", { id: "review" });
    expect(sonner.warning).toHaveBeenLastCalledWith("review", expect.objectContaining({ duration: 6_000 }));
    notify.success("saved", { id: "result" });
    expect(sonner.success).toHaveBeenLastCalledWith("saved", expect.objectContaining({ duration: 4_500 }));
    notify.dismiss("result"); expect(sonner.dismiss).toHaveBeenLastCalledWith("result");
  });
  it("swiping protected messages hides them, accepts hidden updates, and restores the latest state", () => {
    notify.loading("connecting", { id: "task" });
    const original = sonner.mock.calls.at(-1)![1];
    expect(original).toMatchObject({ dismissible: true, closeButton: false, duration: Infinity });
    expect(original.cancel).toBeNull();
    original.onDismiss({ id: "task" });
    expect(getHiddenNotifications()).toMatchObject([{ id: "task", message: "connecting" }]);
    const count = sonner.mock.calls.length;
    notify.update("task", { message: "parsing 2/10" });
    expect(sonner).toHaveBeenCalledTimes(count);
    notify.showHidden();
    expect(getHiddenNotifications()).toHaveLength(0);
    expect(sonner.mock.calls.at(-1)![0]).toBe("parsing 2/10");
    expect(sonner.mock.calls.at(-1)![1].id).not.toBe("task");
    original.onDismiss({ id: "task" }); // Late swipe animation cannot hide the restored view.
    expect(getHiddenNotifications()).toHaveLength(0);
  });
  it("task actions keep their message until the task publishes its terminal state", () => {
    const onClick = vi.fn();
    notify.loading("parsing", { id: "task", action: { label: "stop", onClick } });
    const event = { defaultPrevented: false, preventDefault: vi.fn(function (this: {defaultPrevented: boolean}) { this.defaultPrevented = true; }) };
    const group = sonner.mock.calls.at(-1)![1].action.props;
    void group.onRun(group.actions[0], event);
    expect(event.preventDefault).toHaveBeenCalledOnce(); expect(onClick).toHaveBeenCalledWith(event);
    expect(sonner.dismiss).not.toHaveBeenCalled();
    notify.hide("task");
    notify.success("complete", { id: "task" });
    expect(getHiddenNotifications()).toHaveLength(0);
    expect(sonner.success).toHaveBeenLastCalledWith("complete", expect.objectContaining({ duration: 4_500, action: null, cancel: null }));
    notify.update("missing", { message: "late" });
    expect(getHiddenNotifications()).toHaveLength(0);
  });
  it("explicitly nonclosable nonloading notifications also hide on swipe", () => {
    notify.info("waiting", { id: "task", dismissible: false });
    sonner.info.mock.calls.at(-1)![1].onDismiss({ id: "task" });
    expect(getHiddenNotifications()).toHaveLength(1);
    notify.dismiss("task");
    expect(getHiddenNotifications()).toHaveLength(0);
  });
  it("preserves legacy action dismissal unless its callback updates the notification", async () => {
    const event = {defaultPrevented: false, preventDefault: vi.fn()};
    notify.info("initial", {id: "result", action: {label: "撤销", onClick: vi.fn()}});
    let group = sonner.info.mock.calls.at(-1)![1].action.props;
    await group.onRun(group.actions[0], event);
    expect(sonner.dismiss).toHaveBeenCalledWith("result");
    vi.mocked(sonner.dismiss).mockClear();
    notify.info("initial", {id: "result", action: {label: "撤销", onClick: () => notify.update("result", {message: "done"})}});
    group = sonner.info.mock.calls.at(-1)![1].action.props;
    await group.onRun(group.actions[0], event);
    expect(sonner.dismiss).not.toHaveBeenCalled();
    expect(sonner.info).toHaveBeenLastCalledWith("done", expect.anything());
  });
});

it("retains a pending async action through hide/restore and rejects repeat clicks", async () => {
  let finish!: () => void;
  const onClick = vi.fn(() => new Promise<void>(resolve => { finish = resolve; }));
  const action = { id: "save", label: "保存", onClick };
  notify.loading("saving", { id: "task", actions: [action] });
  const pending = sonner.mock.calls.at(-1)![1].action.props.onRun(action);
  notify.hide("task"); notify.showHidden();
  const restored = sonner.mock.calls.at(-1)![1].action.props;
  expect(restored.pendingActionId).toBe("save");
  await restored.onRun(action);
  expect(onClick).toHaveBeenCalledTimes(1);
  finish(); await pending;
  expect(sonner.mock.calls.at(-1)![1].action.props.pendingActionId).toBeUndefined();
});
