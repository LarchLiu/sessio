import { useCallback, useEffect, useRef, type KeyboardEvent, type RefObject } from "react";

export function useComposerInputHistory({
  entries,
  value,
  setValue,
  textareaRef,
}: {
  entries: string[];
  value: string;
  setValue: (nextValue: string) => void;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
}) {
  const entriesRef = useRef(entries);
  const valueRef = useRef(value);
  const cursorRef = useRef<number | null>(null);
  const draftRef = useRef("");

  useEffect(() => {
    entriesRef.current = entries;
  }, [entries]);

  useEffect(() => {
    valueRef.current = value;
  }, [value]);

  const reset = useCallback(() => {
    cursorRef.current = null;
    draftRef.current = "";
  }, []);

  const applyValue = useCallback((nextValue: string) => {
    setValue(nextValue);
    valueRef.current = nextValue;
    window.requestAnimationFrame(() => {
      const textarea = textareaRef.current;
      if (!textarea) return;
      textarea.focus();
      textarea.setSelectionRange(nextValue.length, nextValue.length);
      resizeTextareaToContent(textarea);
    });
  }, [setValue, textareaRef]);

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (
      event.defaultPrevented ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.shiftKey
    ) {
      return false;
    }
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return false;

    const entries = entriesRef.current;
    const cursor = cursorRef.current;

    if (event.key === "ArrowDown") {
      if (cursor === null) return false;
      event.preventDefault();
      if (cursor <= 0) {
        cursorRef.current = null;
        applyValue(draftRef.current);
        draftRef.current = "";
        return true;
      }
      const nextCursor = cursor - 1;
      cursorRef.current = nextCursor;
      applyValue(entries[nextCursor] ?? "");
      return true;
    }

    if (entries.length === 0) return false;
    if (cursor === null && !caretIsOnFirstLine(event.currentTarget)) return false;
    const nextCursor = cursor === null ? 0 : Math.min(cursor + 1, entries.length - 1);
    if (cursor === null) draftRef.current = valueRef.current;
    cursorRef.current = nextCursor;
    event.preventDefault();
    applyValue(entries[nextCursor] ?? "");
    return true;
  }, [applyValue]);

  return {
    onKeyDown,
    reset,
  };
}

function caretIsOnFirstLine(textarea: HTMLTextAreaElement): boolean {
  if (textarea.selectionStart !== textarea.selectionEnd) return false;
  return textarea.value.lastIndexOf("\n", Math.max(0, textarea.selectionStart - 1)) < 0;
}

function resizeTextareaToContent(textarea: HTMLTextAreaElement): void {
  textarea.style.height = "auto";
  textarea.style.height = `${textarea.scrollHeight}px`;
}
