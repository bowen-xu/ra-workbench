// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {registerNoteOperationLogActions, useNoteOperationLogActions, type NoteOperationLogActions} from "./noteOperationLogActions";

let container: HTMLDivElement, root: ReturnType<typeof createRoot>;
const cleanups: (() => void)[] = [];
function actions(name: string): NoteOperationLogActions & {readonly name: string} {
  return {name, copy: vi.fn(async () => {}), download: vi.fn(async () => {}), clear: vi.fn(), copying: false, downloading: false};
}
function Probe({pageId}: {readonly pageId: string | undefined}): React.JSX.Element {
  const port = useNoteOperationLogActions(pageId) as ReturnType<typeof actions> | undefined;
  return <output>{port?.name ?? "unavailable"}</output>;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => {
  await act(async () => {root.unmount(); cleanups.splice(0).forEach(cleanup => cleanup());});
  container.remove(); vi.unstubAllGlobals();
});

it("selects the exact page and releases only the registered owner", async () => {
  const first = actions("first"), second = actions("second");
  await act(async () => root.render(<Probe pageId="page-a"/>));
  expect(container.textContent).toBe("unavailable");
  let stopFirst = () => {};
  await act(async () => {stopFirst = registerNoteOperationLogActions("page-a", first); cleanups.push(stopFirst, registerNoteOperationLogActions("page-b", second));});
  expect(container.textContent).toBe("first");
  await act(async () => root.render(<Probe pageId="page-b"/>));
  expect(container.textContent).toBe("second");
  await act(async () => {stopFirst(); stopFirst();});
  expect(container.textContent).toBe("second");
  await act(async () => root.render(<Probe pageId={undefined}/>));
  expect(container.textContent).toBe("unavailable");
});

it("hides ambiguous owners including duplicate registrations of the same commands", async () => {
  const port = actions("unique"); let stopFirst = () => {}, stopSecond = () => {};
  await act(async () => {
    root.render(<Probe pageId="page"/>);
    stopFirst = registerNoteOperationLogActions("page", port); cleanups.push(stopFirst);
  });
  expect(container.textContent).toBe("unique");
  await act(async () => {stopSecond = registerNoteOperationLogActions("page", port); cleanups.push(stopSecond);});
  expect(container.textContent).toBe("unavailable");
  await act(async () => stopFirst());
  expect(container.textContent).toBe("unique");
  await act(async () => stopFirst());
  expect(container.textContent).toBe("unique");
  await act(async () => stopSecond());
  expect(container.textContent).toBe("unavailable");
});
