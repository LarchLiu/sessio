import { useEffect, useMemo, useState, type ReactNode, type CSSProperties } from "react";
import { createHighlighterCore, type HighlighterCore, type ThemedToken } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import cssLang from "shiki/langs/css.mjs";
import htmlLang from "shiki/langs/html.mjs";
import javascriptLang from "shiki/langs/javascript.mjs";
import jsonLang from "shiki/langs/json.mjs";
import jsoncLang from "shiki/langs/jsonc.mjs";
import jsxLang from "shiki/langs/jsx.mjs";
import markdownLang from "shiki/langs/markdown.mjs";
import pythonLang from "shiki/langs/python.mjs";
import rustLang from "shiki/langs/rust.mjs";
import shellLang from "shiki/langs/shellscript.mjs";
import tsxLang from "shiki/langs/tsx.mjs";
import typescriptLang from "shiki/langs/typescript.mjs";
import xmlLang from "shiki/langs/xml.mjs";
import yamlLang from "shiki/langs/yaml.mjs";
import githubDarkTheme from "shiki/themes/github-dark.mjs";
import githubLightTheme from "shiki/themes/github-light.mjs";

export type ShikiHighlightedLine = ThemedToken[];

let shikiHighlighterPromise: Promise<HighlighterCore> | null = null;

export function getShikiHighlighter(): Promise<HighlighterCore> {
  shikiHighlighterPromise ??= createHighlighterCore({
    themes: [githubLightTheme, githubDarkTheme],
    langs: [
      cssLang,
      htmlLang,
      javascriptLang,
      jsonLang,
      jsoncLang,
      jsxLang,
      markdownLang,
      pythonLang,
      rustLang,
      shellLang,
      tsxLang,
      typescriptLang,
      xmlLang,
      yamlLang,
    ],
    engine: createJavaScriptRegexEngine(),
  });
  return shikiHighlighterPromise;
}

export function shikiTheme(themeType: "light" | "dark"): string {
  return themeType === "light" ? "github-light" : "github-dark";
}

export function shikiLanguage(language: string): string {
  return shikiLanguageAlias(language) ?? "shell";
}

export function shikiLanguageAlias(language: string): string | null {
  const normalized = language.trim().toLowerCase();
  if (!normalized) return null;
  const aliases: Record<string, string> = {
    bash: "shell",
    shell: "shell",
    sh: "shell",
    zsh: "shell",
    js: "javascript",
    jsx: "jsx",
    ts: "typescript",
    tsx: "tsx",
    py: "python",
    rs: "rust",
    md: "markdown",
    yml: "yaml",
  };
  const supported = new Set([
    "css",
    "html",
    "javascript",
    "json",
    "jsonc",
    "jsx",
    "markdown",
    "python",
    "rust",
    "shell",
    "tsx",
    "typescript",
    "xml",
    "yaml",
  ]);
  const mapped = aliases[normalized] ?? normalized;
  return supported.has(mapped) ? mapped : null;
}

export function languageFromPath(path: string): string {
  const lower = path.toLowerCase();
  const dot = lower.lastIndexOf(".");
  if (dot < 0 || dot === lower.length - 1) return "";
  const ext = lower.slice(dot + 1);
  return ext;
}

export function useShikiHighlightedLines(
  code: string,
  language: string,
  themeType: "light" | "dark",
): ShikiHighlightedLine[] | null {
  const [lines, setLines] = useState<ShikiHighlightedLine[] | null>(null);
  useEffect(() => {
    let cancelled = false;
    const lang = shikiLanguage(language);
    const theme = shikiTheme(themeType);
    setLines(null);
    getShikiHighlighter()
      .then((highlighter) => {
        const tokens = highlighter.codeToTokensBase(code, { lang, theme });
        if (!cancelled) setLines(tokens);
      })
      .catch((err) => {
        console.error("highlight code block failed", err);
        if (!cancelled) setLines(null);
      });
    return () => {
      cancelled = true;
    };
  }, [code, language, themeType]);
  return lines;
}

export function useShikiHighlightedCode(
  code: string,
  language: string,
  themeType: "light" | "dark",
): ReactNode[] | null {
  const lines = useShikiHighlightedLines(code, language, themeType);
  return useMemo(() => {
    if (!lines) return null;
    return renderShikiLines(lines);
  }, [lines]);
}

export function renderShikiLine(line: ShikiHighlightedLine, lineIndex: number): ReactNode {
  if (!line.length) return null;
  return line.map((token, tokenIndex) => (
    <span
      key={`${lineIndex}-${tokenIndex}`}
      style={{
        color: token.color,
        fontStyle: shikiFontStyle(token.fontStyle),
        fontWeight: token.fontStyle !== undefined && (token.fontStyle & 2) ? 600 : undefined,
        textDecorationLine:
          token.fontStyle !== undefined && (token.fontStyle & 4) ? "underline" : undefined,
      }}
    >
      {token.content}
    </span>
  ));
}

export function renderShikiLines(lines: ShikiHighlightedLine[]): ReactNode[] {
  const nodes: ReactNode[] = [];
  lines.forEach((line, lineIndex) => {
    if (lineIndex > 0) nodes.push("\n");
    if (!line.length) return;
    line.forEach((token, tokenIndex) => {
      nodes.push(
        <span
          key={`${lineIndex}-${tokenIndex}`}
          style={{
            color: token.color,
            fontStyle: shikiFontStyle(token.fontStyle),
            fontWeight: token.fontStyle !== undefined && (token.fontStyle & 2) ? 600 : undefined,
            textDecorationLine:
              token.fontStyle !== undefined && (token.fontStyle & 4) ? "underline" : undefined,
          }}
        >
          {token.content}
        </span>,
      );
    });
  });
  return nodes;
}

export function shikiFontStyle(fontStyle?: number): CSSProperties["fontStyle"] {
  return fontStyle !== undefined && (fontStyle & 1) ? "italic" : undefined;
}

export function useEffectiveThemeType(): "light" | "dark" {
  const [themeType, setThemeType] = useState<"light" | "dark">(() =>
    typeof document !== "undefined" && document.documentElement.getAttribute("data-theme") === "light"
      ? "light"
      : "dark",
  );
  useEffect(() => {
    if (typeof document === "undefined") return;
    const root = document.documentElement;
    const update = () => {
      setThemeType(root.getAttribute("data-theme") === "light" ? "light" : "dark");
    };
    update();
    const observer = new MutationObserver(update);
    observer.observe(root, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => observer.disconnect();
  }, []);
  return themeType;
}
