import ScrollArea from "./ScrollArea";
import {
  renderShikiLine,
  shikiLanguageAlias,
  useEffectiveThemeType,
  useShikiHighlightedLines,
} from "./shikiHighlight";

const LARGE_FILE_HIGHLIGHT_BYTES = 200_000;
const GUTTER_CHAR_PX = 8; // monospace digit width approximation in caption font
const GUTTER_PADDING_PX = 16;

export interface FileViewerProps {
  text: string;
  /** Raw extension or language hint (e.g. "ts", "rs", "json"). */
  language: string;
  mode: "code" | "plain";
}

export default function FileViewer({ text, language, mode }: FileViewerProps) {
  const themeType = useEffectiveThemeType();
  const resolvedLanguage = shikiLanguageAlias(language) ?? "";
  const tooLargeToHighlight = text.length > LARGE_FILE_HIGHLIGHT_BYTES;
  const wantHighlight = mode === "code" && Boolean(resolvedLanguage) && !tooLargeToHighlight;
  const highlightedLines = useShikiHighlightedLines(
    wantHighlight ? text : "",
    resolvedLanguage,
    themeType,
  );

  const useHighlight = wantHighlight && highlightedLines !== null;
  const lineCount = useHighlight ? (highlightedLines?.length ?? 0) : text.split("\n").length;
  const gutterDigits = String(Math.max(2, lineCount)).length;
  const gutterWidthPx = gutterDigits * GUTTER_CHAR_PX + GUTTER_PADDING_PX;

  return (
    <ScrollArea
      className="h-full min-h-0 min-w-0"
      viewportClassName="px-10 py-4"
      persistScrollbars
    >
      {useHighlight ? (
        <div>
          {highlightedLines?.map((line, index) => (
            <div key={index} className="flex font-mono text-caption leading-relaxed">
              <span
                aria-hidden
                className="select-none shrink-0 pr-3 text-right text-ink/30 tabular-nums"
                style={{ width: gutterWidthPx }}
              >
                {index + 1}
              </span>
              <span className="min-w-0 flex-1 whitespace-pre-wrap break-words">
                {renderShikiLine(line, index) ?? " "}
              </span>
            </div>
          ))}
        </div>
      ) : (
        <pre className="whitespace-pre-wrap break-words font-mono text-caption leading-relaxed text-ink/82">
          {text}
        </pre>
      )}
    </ScrollArea>
  );
}
