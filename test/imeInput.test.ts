import { describe, expect, it } from "vitest";
import {
  createImeCompositionState,
  getImeKeyboardDisposition,
  markImeCompositionEnd,
  markImeCompositionStart,
} from "../src/components/imeInput";

describe("IME keyboard shortcut guard", () => {
  it("skips shortcuts while composition is active", () => {
    const state = createImeCompositionState();
    markImeCompositionStart(state);

    expect(getImeKeyboardDisposition({ key: "Enter" }, state, 100)).toEqual({
      shouldSkipShortcut: true,
      shouldPreventDefault: false,
    });
  });

  it("skips shortcuts for native composing events", () => {
    const state = createImeCompositionState();

    expect(
      getImeKeyboardDisposition(
        { key: "Enter", nativeEvent: { isComposing: true } },
        state,
        100,
      ),
    ).toEqual({
      shouldSkipShortcut: true,
      shouldPreventDefault: false,
    });
  });

  it("skips shortcuts for process-key events reported as keyCode 229", () => {
    const state = createImeCompositionState();

    expect(
      getImeKeyboardDisposition(
        { key: "Enter", nativeEvent: { keyCode: 229 } },
        state,
        100,
      ),
    ).toEqual({
      shouldSkipShortcut: true,
      shouldPreventDefault: false,
    });
  });

  it("suppresses a plain Enter leaked immediately after composition ends", () => {
    const state = createImeCompositionState();
    markImeCompositionEnd(state, 100);

    expect(getImeKeyboardDisposition({ key: "Enter" }, state, 120)).toEqual({
      shouldSkipShortcut: true,
      shouldPreventDefault: true,
    });
  });

  it("allows Enter after the composition-end grace period", () => {
    const state = createImeCompositionState();
    markImeCompositionEnd(state, 100);

    expect(getImeKeyboardDisposition({ key: "Enter" }, state, 220)).toEqual({
      shouldSkipShortcut: false,
      shouldPreventDefault: false,
    });
  });

  it("allows modified Enter after composition ends", () => {
    const state = createImeCompositionState();
    markImeCompositionEnd(state, 100);

    expect(getImeKeyboardDisposition({ key: "Enter", shiftKey: true }, state, 120)).toEqual({
      shouldSkipShortcut: false,
      shouldPreventDefault: false,
    });
  });
});
