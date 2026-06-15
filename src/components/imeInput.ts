export const IME_COMPOSITION_END_ENTER_GRACE_MS = 80;

export type ImeCompositionState = {
  isComposing: boolean;
  lastCompositionEndAt: number;
};

type NativeKeyboardLike = {
  isComposing?: boolean;
  keyCode?: number;
  which?: number;
};

type KeyboardEventLike = {
  key: string;
  shiftKey?: boolean;
  altKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
  nativeEvent?: NativeKeyboardLike;
};

export type ImeKeyboardDisposition = {
  shouldSkipShortcut: boolean;
  shouldPreventDefault: boolean;
};

export function createImeCompositionState(): ImeCompositionState {
  return {
    isComposing: false,
    lastCompositionEndAt: Number.NEGATIVE_INFINITY,
  };
}

export function markImeCompositionStart(state: ImeCompositionState): void {
  state.isComposing = true;
}

export function markImeCompositionEnd(
  state: ImeCompositionState,
  now = currentTimeMs(),
): void {
  state.isComposing = false;
  state.lastCompositionEndAt = now;
}

export function getImeKeyboardDisposition(
  event: KeyboardEventLike,
  state: ImeCompositionState,
  now = currentTimeMs(),
): ImeKeyboardDisposition {
  const nativeEvent = event.nativeEvent;
  if (
    state.isComposing ||
    nativeEvent?.isComposing ||
    nativeEvent?.keyCode === 229 ||
    nativeEvent?.which === 229
  ) {
    return {
      shouldSkipShortcut: true,
      shouldPreventDefault: false,
    };
  }

  // Some WebView IMEs emit a plain Enter immediately after compositionend.
  if (isPlainEnter(event) && now - state.lastCompositionEndAt <= IME_COMPOSITION_END_ENTER_GRACE_MS) {
    return {
      shouldSkipShortcut: true,
      shouldPreventDefault: true,
    };
  }

  return {
    shouldSkipShortcut: false,
    shouldPreventDefault: false,
  };
}

function isPlainEnter(event: KeyboardEventLike): boolean {
  return (
    event.key === "Enter" &&
    !event.shiftKey &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey
  );
}

function currentTimeMs(): number {
  return typeof performance === "undefined" ? Date.now() : performance.now();
}
