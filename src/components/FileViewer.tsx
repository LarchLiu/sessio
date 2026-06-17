import { useEffect, useMemo, useRef } from "react";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { defaultHighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { oneDark } from "@codemirror/theme-one-dark";
import { cssLanguage } from "@codemirror/lang-css";
import { htmlLanguage } from "@codemirror/lang-html";
import { javascriptLanguage, jsxLanguage, tsxLanguage, typescriptLanguage } from "@codemirror/lang-javascript";
import { jsonLanguage } from "@codemirror/lang-json";
import { markdownLanguage } from "@codemirror/lang-markdown";
import { pythonLanguage } from "@codemirror/lang-python";
import { rustLanguage } from "@codemirror/lang-rust";
import { xmlLanguage } from "@codemirror/lang-xml";
import { yamlLanguage } from "@codemirror/lang-yaml";
import { StreamLanguage } from "@codemirror/language";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import ScrollArea from "./ScrollArea";
import { useEffectiveThemeType } from "./shikiHighlight";

const languageCompartment = new Compartment();
const themeCompartment = new Compartment();

const baseTheme = EditorView.theme({
  "&": {
    height: "100%",
    backgroundColor: "transparent",
    color: "rgb(var(--color-fg) / 0.82)",
    fontFamily:
      'ui-monospace, "SFMono-Regular", "SF Mono", Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
    fontSize: "12px",
  },
  ".cm-scroller": {
    overflow: "visible",
    fontFamily: "inherit",
    lineHeight: "1.6",
  },
  ".cm-content": {
    padding: "0",
    minHeight: "100%",
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  },
  ".cm-line": {
    padding: "0",
  },
  ".cm-gutters": {
    backgroundColor: "transparent",
    border: "none",
    color: "rgb(var(--color-fg) / 0.30)",
    marginRight: "12px",
  },
  ".cm-lineNumbers .cm-gutterElement": {
    padding: "0 12px 0 0",
  },
  ".cm-activeLine, .cm-activeLineGutter": {
    backgroundColor: "transparent",
  },
  ".cm-selectionBackground, ::selection": {
    backgroundColor: "rgb(var(--color-fg) / 0.14) !important",
  },
  ".cm-focused": {
    outline: "none",
  },
  ".cm-cursor, .cm-dropCursor": {
    display: "none",
  },
});

const lightTheme = EditorView.theme({
  "&": {
    color: "rgb(var(--color-fg) / 0.82)",
  },
});

const plainTheme = EditorView.theme({
  ".cm-content": {
    whiteSpace: "pre-wrap",
  },
});

export interface FileViewerProps {
  fileKey: string;
  text: string;
  language: string;
  mode: "code" | "plain";
  savedScrollTop?: number;
  onScrollTopChange?: (scrollTop: number) => void;
}

export default function FileViewer({
  fileKey,
  text,
  language,
  mode,
  savedScrollTop = 0,
  onScrollTopChange,
}: FileViewerProps) {
  const themeType = useEffectiveThemeType();
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const scrollViewportRef = useRef<HTMLDivElement | null>(null);
  const lastAppliedFileKeyRef = useRef<string>("");
  const codeExtension = useMemo<Extension>(
    () => (mode === "code" ? languageExtension(language) : []),
    [language, mode],
  );
  const activeTheme = useMemo<Extension[]>(() => {
    const extensions: Extension[] = [baseTheme];
    if (mode === "plain") extensions.push(plainTheme);
    extensions.push(themeType === "dark" ? oneDark : lightTheme);
    return extensions;
  }, [mode, themeType]);

  useEffect(() => {
    if (!hostRef.current) return;
    const state = EditorState.create({
      doc: text,
      extensions: [
        EditorView.editable.of(false),
        EditorState.readOnly.of(true),
        EditorView.lineWrapping,
        lineNumbers(),
        keymap.of([]),
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        languageCompartment.of(codeExtension),
        themeCompartment.of(activeTheme),
      ],
    });
    const view = new EditorView({
      state,
      parent: hostRef.current,
    });
    viewRef.current = view;
    scrollViewportRef.current = findScrollableParent(hostRef.current);
    const viewport = scrollViewportRef.current;
    if (viewport) {
      viewport.scrollTop = savedScrollTop;
      lastAppliedFileKeyRef.current = fileKey;
    }
    return () => {
      view.destroy();
      viewRef.current = null;
      scrollViewportRef.current = null;
    };
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current !== text) {
      view.dispatch({
        changes: { from: 0, to: current.length, insert: text },
      });
    }
  }, [text]);

  useEffect(() => {
    const viewport = scrollViewportRef.current;
    if (!viewport) return;
    if (lastAppliedFileKeyRef.current === fileKey) return;
    viewport.scrollTop = savedScrollTop;
    lastAppliedFileKeyRef.current = fileKey;
  }, [fileKey, savedScrollTop]);

  useEffect(() => {
    const viewport = scrollViewportRef.current;
    if (!viewport || !onScrollTopChange) return;
    const handleScroll = () => onScrollTopChange(viewport.scrollTop);
    viewport.addEventListener("scroll", handleScroll, { passive: true });
    return () => viewport.removeEventListener("scroll", handleScroll);
  }, [onScrollTopChange, fileKey]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: languageCompartment.reconfigure(mode === "code" ? languageExtension(language) : []),
    });
  }, [codeExtension, language, mode]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: themeCompartment.reconfigure(activeTheme),
    });
  }, [activeTheme]);

  return (
    <ScrollArea
      className="h-full min-h-0 min-w-0"
      viewportClassName="px-10 py-4"
      persistScrollbars
    >
      <div ref={hostRef} className="min-h-full" />
    </ScrollArea>
  );
}

function findScrollableParent(node: HTMLElement): HTMLDivElement | null {
  let current: HTMLElement | null = node.parentElement;
  while (current) {
    if (current instanceof HTMLDivElement && current.scrollHeight >= current.clientHeight) {
      const style = window.getComputedStyle(current);
      if (style.overflowY === "auto" || style.overflowY === "scroll") {
        return current;
      }
    }
    current = current.parentElement;
  }
  return null;
}

function languageExtension(language: string): Extension {
  const normalized = language.trim().toLowerCase();
  switch (normalized) {
    case "js":
    case "javascript":
      return javascriptLanguage;
    case "jsx":
      return jsxLanguage;
    case "ts":
    case "typescript":
      return typescriptLanguage;
    case "tsx":
      return tsxLanguage;
    case "json":
      return jsonLanguage;
    case "jsonc":
      return jsonLanguage;
    case "md":
    case "markdown":
      return markdownLanguage;
    case "py":
    case "python":
      return pythonLanguage;
    case "rs":
    case "rust":
      return rustLanguage;
    case "html":
      return htmlLanguage;
    case "css":
      return cssLanguage;
    case "xml":
      return xmlLanguage;
    case "yaml":
    case "yml":
      return yamlLanguage;
    case "sh":
    case "bash":
    case "shell":
    case "zsh":
      return StreamLanguage.define(shell);
    default:
      return [];
  }
}
