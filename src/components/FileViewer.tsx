import { useEffect, useMemo, useRef } from "react";
import { Compartment, EditorState, RangeSetBuilder, StateEffect, StateField, type Extension } from "@codemirror/state";
import { Decoration, EditorView, GutterMarker, WidgetType, gutter, keymap, lineNumbers } from "@codemirror/view";
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
import type { FileGitDiff } from "../api";
import { useEffectiveThemeType } from "./shikiHighlight";

const languageCompartment = new Compartment();
const themeCompartment = new Compartment();
const diffCompartment = new Compartment();

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
    overflow: "auto",
    fontFamily: "inherit",
    lineHeight: "1.6",
    padding: "16px 40px",
    scrollbarWidth: "thin",
    scrollbarColor: "rgb(var(--color-fg) / 0.28) transparent",
  },
  ".cm-scroller::-webkit-scrollbar": {
    width: "8px",
    height: "8px",
  },
  ".cm-scroller::-webkit-scrollbar-track": {
    backgroundColor: "transparent",
  },
  ".cm-scroller::-webkit-scrollbar-thumb": {
    borderRadius: "999px",
    backgroundColor: "rgb(var(--color-fg) / 0.28)",
  },
  ".cm-scroller::-webkit-scrollbar-thumb:hover": {
    backgroundColor: "rgb(var(--color-fg) / 0.42)",
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
  ".cm-git-diff-gutter": {
    width: "8px",
    minWidth: "8px",
  },
  ".cm-git-diff-gutter .cm-gutterElement": {
    minWidth: "8px",
    padding: "0",
    position: "relative",
  },
  ".cm-git-diff-gutter .cm-gutterElement.cm-git-marker-added, .cm-git-diff-gutter .cm-gutterElement.cm-git-marker-modified, .cm-git-diff-gutter .cm-gutterElement.cm-git-marker-deleted": {
    cursor: "pointer",
  },
  ".cm-git-diff-gutter .cm-gutterElement.cm-git-marker-added:hover::before, .cm-git-diff-gutter .cm-gutterElement.cm-git-marker-modified:hover::before": {
    width: "4px",
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
  ".cm-line.cm-git-line-added": {
    backgroundColor: "rgb(var(--color-emerald) / 0.10)",
  },
  ".cm-line.cm-git-line-modified": {
    backgroundColor: "rgb(var(--color-blue) / 0.10)",
  },
  ".cm-git-marker-spacer": {
    display: "block",
    width: "8px",
  },
  ".cm-git-diff-gutter .cm-gutterElement.cm-git-marker-added::before, .cm-git-diff-gutter .cm-gutterElement.cm-git-marker-modified::before": {
    content: '""',
    position: "absolute",
    left: "2px",
    top: "2px",
    bottom: "2px",
    width: "3px",
    borderRadius: "999px",
  },
  ".cm-git-diff-gutter .cm-gutterElement.cm-git-marker-added::before": {
    backgroundColor: "rgb(var(--color-emerald))",
  },
  ".cm-git-diff-gutter .cm-gutterElement.cm-git-marker-modified::before": {
    backgroundColor: "rgb(var(--color-blue))",
  },
  ".cm-git-diff-gutter .cm-gutterElement.cm-git-marker-deleted::before": {
    content: '""',
    position: "absolute",
    left: "0",
    right: "1px",
    top: "-1px",
    height: "2px",
    borderRadius: "999px",
    backgroundColor: "rgb(var(--color-status-error))",
  },
  ".cm-git-diff-gutter .cm-gutterElement.cm-git-marker-deleted::after": {
    content: '""',
    position: "absolute",
    left: "0",
    top: "-4px",
    width: "0",
    height: "0",
    borderTop: "4px solid transparent",
    borderBottom: "4px solid transparent",
    borderLeft: "5px solid rgb(var(--color-status-error))",
  },
  ".cm-git-diff-preview": {
    maxHeight: "360px",
    overflow: "auto",
    margin: "3px 0 5px 0",
    border: "1px solid rgb(var(--color-fg) / 0.10)",
    borderLeft: "3px solid rgb(var(--color-status-error) / 0.75)",
    borderRadius: "7px",
    backgroundColor: "rgb(var(--color-bg-panel-alt) / 0.96)",
    boxShadow: "0 8px 24px rgb(0 0 0 / 0.16)",
    scrollbarWidth: "thin",
    scrollbarColor: "rgb(var(--color-fg) / 0.28) transparent",
  },
  ".cm-git-diff-preview-row": {
    display: "flex",
    minHeight: "1.6em",
    whiteSpace: "pre-wrap",
  },
  ".cm-git-diff-preview-row-old": {
    backgroundColor: "rgb(var(--color-status-error) / 0.12)",
  },
  ".cm-git-diff-preview-row-new": {
    backgroundColor: "rgb(var(--color-emerald) / 0.12)",
  },
  ".cm-git-diff-preview-prefix": {
    flex: "0 0 22px",
    textAlign: "center",
    userSelect: "none",
    opacity: "0.82",
  },
  ".cm-git-diff-preview-row-old .cm-git-diff-preview-prefix": {
    color: "rgb(var(--color-status-error))",
  },
  ".cm-git-diff-preview-row-new .cm-git-diff-preview-prefix": {
    color: "rgb(var(--color-emerald))",
  },
  ".cm-git-diff-preview-code": {
    minWidth: "0",
    flex: "1 1 auto",
    paddingRight: "12px",
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
  gitDiff?: FileGitDiff | null;
  savedScrollTop?: number;
  onScrollTopChange?: (scrollTop: number) => void;
}

export default function FileViewer({
  fileKey,
  text,
  language,
  mode,
  gitDiff = null,
  savedScrollTop = 0,
  onScrollTopChange,
}: FileViewerProps) {
  const themeType = useEffectiveThemeType();
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const scrollViewportRef = useRef<HTMLElement | null>(null);
  const lastAppliedFileKeyRef = useRef<string>("");
  const codeExtension = useMemo<Extension>(
    () => (mode === "code" ? languageExtension(language) : []),
    [language, mode],
  );
  const diffExtension = useMemo<Extension>(
    () => buildGitDiffExtension(text, gitDiff),
    [gitDiff, text],
  );
  const diffOverviewMarks = useMemo(
    () => buildDiffOverviewMarks(text, gitDiff),
    [gitDiff, text],
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
        diffCompartment.of(diffExtension),
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
    scrollViewportRef.current = view.scrollDOM;
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

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: [
        diffCompartment.reconfigure(diffExtension),
        setDiffPreviewBlockEffect.of(null),
      ],
    });
  }, [diffExtension]);

  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      const view = viewRef.current;
      if (!view || !view.state.field(diffPreviewField, false)) return;
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (target.closest(".cm-git-diff-preview, .cm-git-diff-gutter")) return;
      view.dispatch({
        effects: setDiffPreviewBlockEffect.of(null),
      });
    };
    document.addEventListener("pointerdown", handlePointerDown, true);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown, true);
    };
  }, []);

  return (
    <div className="relative h-full min-h-0 min-w-0">
      <div ref={hostRef} className="h-full min-h-0 min-w-0" />
      {diffOverviewMarks.length > 0 && (
        <div className="pointer-events-none absolute bottom-4 right-3 top-4 z-20 w-2">
          <div className="absolute inset-y-0 right-0 w-px rounded-full bg-ink/[0.08]" />
          {diffOverviewMarks.map((mark, index) => (
            <span
              key={`${mark.kind}-${mark.startLine}-${index}`}
              className="absolute right-0 w-1.5 rounded-full"
              style={{
                top: `${mark.topPct}%`,
                height: `${mark.heightPct}%`,
                minHeight: 2,
                backgroundColor: overviewMarkColor(mark.kind),
              }}
            />
          ))}
        </div>
      )}
    </div>
  );
}

type GitLineKind = "added" | "modified" | "deleted";

interface DiffOverviewMark {
  kind: GitLineKind;
  startLine: number;
  topPct: number;
  heightPct: number;
}

interface GitDiffBlock {
  id: string;
  anchorLine: number;
  oldLines: string[];
  newLines: string[];
}

interface ResolvedGitDiffBlock extends GitDiffBlock {
  anchorPos: number;
}

interface ParsedGitDecorations {
  lineKinds: Map<number, GitLineKind>;
  lineBlockIds: Map<number, string>;
  blocks: GitDiffBlock[];
}

const setDiffPreviewBlockEffect = StateEffect.define<ResolvedGitDiffBlock | null>();

const diffPreviewField = StateField.define<ResolvedGitDiffBlock | null>({
  create: () => null,
  update(value, transaction) {
    let next = transaction.docChanged ? null : value;
    for (const effect of transaction.effects) {
      if (effect.is(setDiffPreviewBlockEffect)) {
        next = effect.value;
      }
    }
    return next;
  },
  provide: (field) =>
    EditorView.decorations.from(field, (block) => {
      if (!block) return Decoration.none;
      const builder = new RangeSetBuilder<Decoration>();
      builder.add(
        block.anchorPos,
        block.anchorPos,
        Decoration.widget({
          widget: new GitDiffPreviewWidget(block),
          block: true,
          side: -1,
        }),
      );
      return builder.finish();
    }),
});

function buildGitDiffExtension(text: string, gitDiff: FileGitDiff | null): Extension {
  if (!gitDiff || !gitDiff.patch || !gitDiff.patch.trim()) return [];
  const parsed = parseGitPatch(text, gitDiff);
  if (parsed.lineKinds.size === 0 || parsed.blocks.length === 0) return [];

  const state = EditorState.create({ doc: text });
  const resolvedBlocks = new Map<string, ResolvedGitDiffBlock>();
  for (const block of parsed.blocks) {
    const anchorLine = safeLine(state, Math.min(block.anchorLine, state.doc.lines));
    resolvedBlocks.set(block.id, {
      ...block,
      anchorPos: anchorLine?.from ?? state.doc.length,
    });
  }
  const markerBuilder = new RangeSetBuilder<GutterMarker>();
  const decorationBuilder = new RangeSetBuilder<Decoration>();
  const markerRanges: Array<{ from: number; marker: GutterMarker }> = [];
  const decorationRanges: Array<{ from: number; to: number; decoration: Decoration }> = [];

  for (const [lineNumber, kind] of parsed.lineKinds) {
    const line = safeLine(state, lineNumber);
    if (!line) continue;
    const marker = gitMarker(kind);
    markerRanges.push({ from: line.from, marker });
    if (kind === "deleted") continue;
    decorationRanges.push({
      from: line.from,
      to: line.from,
      decoration: Decoration.line({
        class: kind === "added" ? "cm-git-line-added" : "cm-git-line-modified",
      }),
    });
  }

  markerRanges.sort((a, b) => a.from - b.from || a.marker.startSide - b.marker.startSide);
  for (const range of markerRanges) {
    markerBuilder.add(range.from, range.from, range.marker);
  }
  const markerSet = markerBuilder.finish();

  decorationRanges.sort(
    (a, b) =>
      a.from - b.from ||
      a.decoration.startSide - b.decoration.startSide ||
      a.to - b.to ||
      a.decoration.endSide - b.decoration.endSide,
  );
  for (const range of decorationRanges) {
    decorationBuilder.add(range.from, range.to, range.decoration);
  }

  return [
    diffPreviewField,
    gutter({
      class: "cm-git-diff-gutter",
      markers: () => markerSet,
      initialSpacer: () => GIT_GUTTER_SPACER,
      domEventHandlers: {
        click(view, line, event) {
          const lineNumber = view.state.doc.lineAt(line.from).number;
          const blockId = parsed.lineBlockIds.get(lineNumber);
          const block = blockId ? resolvedBlocks.get(blockId) : null;
          if (!block) return false;
          const current = view.state.field(diffPreviewField, false);
          view.dispatch({
            effects: setDiffPreviewBlockEffect.of(
              current?.id === block.id ? null : block,
            ),
          });
          event.preventDefault();
          return true;
        },
      },
    }),
    EditorView.decorations.of(decorationBuilder.finish()),
  ];
}

function buildDiffOverviewMarks(
  text: string,
  gitDiff: FileGitDiff | null,
): DiffOverviewMark[] {
  if (!gitDiff || !gitDiff.patch || !gitDiff.patch.trim()) return [];
  const parsed = parseGitPatch(text, gitDiff);
  if (parsed.lineKinds.size === 0) return [];
  const totalLines = Math.max(1, text.split("\n").length);
  const sorted = Array.from(parsed.lineKinds.entries()).sort((a, b) => a[0] - b[0]);
  const ranges: Array<{ kind: GitLineKind; startLine: number; endLine: number }> = [];
  for (const [lineNumber, kind] of sorted) {
    const previous = ranges[ranges.length - 1];
    if (previous && previous.kind === kind && lineNumber <= previous.endLine + 1) {
      previous.endLine = lineNumber;
      continue;
    }
    ranges.push({ kind, startLine: lineNumber, endLine: lineNumber });
  }
  return ranges.map((range) => {
    const start = Math.max(1, Math.min(totalLines, range.startLine));
    const end = Math.max(start, Math.min(totalLines, range.endLine));
    return {
      kind: range.kind,
      startLine: start,
      topPct: ((start - 1) / totalLines) * 100,
      heightPct: Math.max(((end - start + 1) / totalLines) * 100, 0.35),
    };
  });
}

function overviewMarkColor(kind: GitLineKind): string {
  if (kind === "added") return "rgb(var(--color-emerald))";
  if (kind === "modified") return "rgb(var(--color-blue))";
  return "rgb(var(--color-status-error))";
}

function safeLine(state: EditorState, lineNumber: number) {
  if (lineNumber < 1 || lineNumber > state.doc.lines) return null;
  return state.doc.line(lineNumber);
}

class GitLineMarker extends GutterMarker {
  readonly elementClass: string;

  constructor(readonly kind: GitLineKind) {
    super();
    this.elementClass = this.kind === "added"
      ? "cm-git-marker-added"
      : this.kind === "modified"
        ? "cm-git-marker-modified"
        : "cm-git-marker-deleted";
  }

  eq(other: GutterMarker) {
    return other instanceof GitLineMarker && other.kind === this.kind;
  }
}

const ADDED_MARKER = new GitLineMarker("added");
const MODIFIED_MARKER = new GitLineMarker("modified");
const DELETED_MARKER = new GitLineMarker("deleted");

class GitGutterSpacer extends GutterMarker {
  eq(other: GutterMarker) {
    return other instanceof GitGutterSpacer;
  }

  toDOM() {
    const spacer = document.createElement("span");
    spacer.className = "cm-git-marker-spacer";
    return spacer;
  }
}

const GIT_GUTTER_SPACER = new GitGutterSpacer();

function gitMarker(kind: GitLineKind) {
  if (kind === "added") return ADDED_MARKER;
  if (kind === "deleted") return DELETED_MARKER;
  return MODIFIED_MARKER;
}

class GitDiffPreviewWidget extends WidgetType {
  constructor(private readonly block: ResolvedGitDiffBlock) {
    super();
  }

  eq(other: WidgetType) {
    return other instanceof GitDiffPreviewWidget &&
      other.block.id === this.block.id &&
      sameLines(other.block.oldLines, this.block.oldLines) &&
      sameLines(other.block.newLines, this.block.newLines);
  }

  toDOM() {
    const root = document.createElement("div");
    root.className = "cm-git-diff-preview";
    for (const line of this.block.oldLines) {
      root.appendChild(renderDiffPreviewRow("-", line, "old"));
    }
    for (const line of this.block.newLines) {
      root.appendChild(renderDiffPreviewRow("+", line, "new"));
    }
    return root;
  }
}

function renderDiffPreviewRow(prefix: "+" | "-", text: string, kind: "old" | "new") {
  const row = document.createElement("div");
  row.className = `cm-git-diff-preview-row cm-git-diff-preview-row-${kind}`;
  const prefixNode = document.createElement("span");
  prefixNode.className = "cm-git-diff-preview-prefix";
  prefixNode.textContent = prefix;
  const codeNode = document.createElement("span");
  codeNode.className = "cm-git-diff-preview-code";
  codeNode.textContent = text || " ";
  row.append(prefixNode, codeNode);
  return row;
}

function sameLines(a: string[], b: string[]) {
  return a.length === b.length && a.every((line, index) => line === b[index]);
}

function parseGitPatch(text: string, gitDiff: FileGitDiff): ParsedGitDecorations {
  if (gitDiff.status === "clean" || !gitDiff.patch) {
    return {
      lineKinds: new Map(),
      lineBlockIds: new Map(),
      blocks: [],
    };
  }

  const documentLines = text.split("\n");
  const maxDocumentLine = Math.max(1, documentLines.length);
  const lineKinds = new Map<number, GitLineKind>();
  const lineBlockIds = new Map<number, string>();
  const blocks: GitDiffBlock[] = [];
  const lines = gitDiff.patch.split("\n");
  let index = 0;
  let blockIndex = 0;

  const setLineKind = (lineNumber: number, kind: GitLineKind) => {
    if (lineNumber < 1 || lineNumber > documentLines.length) return;
    const current = lineKinds.get(lineNumber);
    if (current === "modified" || (current === "added" && kind === "deleted")) {
      return;
    }
    lineKinds.set(lineNumber, kind);
  };

  const setBlockLine = (lineNumber: number, blockId: string) => {
    if (lineNumber < 1 || lineNumber > documentLines.length) return;
    lineBlockIds.set(lineNumber, blockId);
  };

  while (index < lines.length) {
    const header = lines[index];
    const match = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(header);
    if (!match) {
      index += 1;
      continue;
    }

    const newStart = Number(match[3]);
    index += 1;

    let newLineCursor = newStart;
    let pendingOldLines: string[] = [];
    let pendingNewLines: string[] = [];
    let pendingNewLineNumbers: number[] = [];
    let pendingAnchorLine: number | null = null;

    const flushChangeBlock = () => {
      if (pendingOldLines.length === 0 && pendingNewLines.length === 0) return;
      const hasOld = pendingOldLines.length > 0;
      const hasNew = pendingNewLineNumbers.length > 0;
      const validNewLineNumbers = pendingNewLineNumbers.filter(
        (lineNumber) => lineNumber >= 1 && lineNumber <= documentLines.length,
      );
      const fallbackLine = Math.min(
        Math.max(1, pendingAnchorLine ?? newLineCursor),
        maxDocumentLine,
      );
      const markerLines = validNewLineNumbers.length > 0
        ? validNewLineNumbers
        : [fallbackLine];
      const blockId = `${blockIndex}:${markerLines[0]}:${pendingOldLines.length}:${pendingNewLines.length}`;
      blockIndex += 1;
      const markerKind: GitLineKind = hasOld && hasNew
        ? "modified"
        : hasNew
          ? "added"
          : "deleted";
      for (const lineNumber of markerLines) {
        setLineKind(lineNumber, markerKind);
        setBlockLine(lineNumber, blockId);
      }
      blocks.push({
        id: blockId,
        anchorLine: markerLines[0],
        oldLines: pendingOldLines,
        newLines: pendingNewLines,
      });
      pendingOldLines = [];
      pendingNewLines = [];
      pendingNewLineNumbers = [];
      pendingAnchorLine = null;
    };

    while (index < lines.length) {
      const raw = lines[index];
      if (raw.startsWith("@@ ") || raw.startsWith("diff --git ")) {
        flushChangeBlock();
        break;
      }
      if (raw.startsWith("--- ") || raw.startsWith("+++ ") || raw.startsWith("index ") || raw.startsWith("new file mode ")) {
        index += 1;
        continue;
      }
      if (raw === "\\ No newline at end of file") {
        index += 1;
        continue;
      }
      if (raw.startsWith("-")) {
        if (pendingNewLines.length > 0) flushChangeBlock();
        pendingOldLines.push(raw.slice(1));
        index += 1;
        continue;
      }
      if (raw.startsWith("+")) {
        const targetLine = Math.max(1, newLineCursor);
        if (pendingAnchorLine === null) pendingAnchorLine = targetLine;
        pendingNewLines.push(raw.slice(1));
        pendingNewLineNumbers.push(targetLine);
        newLineCursor += 1;
        index += 1;
        continue;
      }
      if (raw.startsWith(" ")) {
        flushChangeBlock();
        newLineCursor += 1;
      }
      index += 1;
    }
    flushChangeBlock();
  }

  return { lineKinds, lineBlockIds, blocks };
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
