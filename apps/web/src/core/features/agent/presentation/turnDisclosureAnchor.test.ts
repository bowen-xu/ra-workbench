/** @vitest-environment happy-dom */
import { afterEach, expect, it, vi } from "vitest";
import { captureTurnDisclosureAnchor, releaseTurnDisclosureAnchor } from "./turnDisclosureAnchor";

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

it("reserves scroll range before collapse, corrects the header, and releases on user scroll", () => {
  const scroller = document.createElement("div");
  scroller.className = "writing-agent-messages";
  const transcript = document.createElement("div");
  transcript.className = "writing-agent-transcript";
  const summary = document.createElement("summary");
  transcript.append(summary); scroller.append(transcript); document.body.append(scroller);
  Object.defineProperties(scroller, { clientHeight: { value: 400 }, scrollHeight: { value: 1028 } });
  Object.defineProperty(transcript, "offsetHeight", { value: 1000 });
  scroller.scrollTop = 500;
  let top = 140;
  vi.spyOn(summary, "getBoundingClientRect").mockImplementation(() => ({ top } as DOMRect));
  const preserve = vi.fn(); scroller.addEventListener("writing-agent-preserve-position", preserve);
  const anchor = captureTurnDisclosureAnchor(summary)!;
  expect(preserve).toHaveBeenCalledOnce();
  expect(transcript.style.minHeight).toBe("max(100%, 880px)");
  top = 160; anchor.restore();
  expect(scroller.scrollTop).toBe(520);
  scroller.dispatchEvent(new Event("scroll"));
  expect(transcript.style.minHeight).not.toBe("");
  // Later programmatic search openings must not restore an old toggle anchor.
  top = 190; anchor.restore();
  expect(scroller.scrollTop).toBe(520);
  scroller.scrollTop = 450; scroller.dispatchEvent(new Event("scroll"));
  expect(transcript.style.minHeight).toBe("");
});

it("replaces the prior reservation and allows the explicit return-to-bottom action", () => {
  const scroller = document.createElement("div"); scroller.className = "writing-agent-messages";
  const transcript = document.createElement("div"); transcript.className = "writing-agent-transcript";
  const summary = document.createElement("summary");
  transcript.style.minHeight = "100%";
  transcript.append(summary); scroller.append(transcript); document.body.append(scroller);
  const first = captureTurnDisclosureAnchor(summary)!;
  const next = captureTurnDisclosureAnchor(summary)!;
  first.release();
  expect(transcript.style.minHeight).not.toBe("100%");
  releaseTurnDisclosureAnchor(scroller);
  expect(transcript.style.minHeight).toBe("100%");
  next.release();
});
