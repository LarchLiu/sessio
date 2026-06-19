import {
  createElement,
  isValidElement,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type HTMLAttributes,
  type ReactNode,
  type StyleHTMLAttributes,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import ReactMarkdown, { type Components } from "react-markdown";
import rehypeKatex from "rehype-katex";
import rehypeRaw from "rehype-raw";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import type { VisualizationSpec } from "vega-embed";
import { readLocalImageDataUrl } from "../api";
import { renderMarkdownInput } from "./markdownInput";
import ScrollArea from "./ScrollArea";
import { useEffectiveThemeType, useShikiHighlightedCode } from "./shikiHighlight";
import "katex/dist/katex.min.css";

type HastNode = {
  type?: string;
  tagName?: string;
  value?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
};

type PreviewThemeType = "light" | "dark";

const blockedHtmlTags = new Set(["script", "iframe", "object", "embed", "audio", "video"]);
const urlAttributeNames = new Set([
  "src",
  "href",
  "xlinkHref",
  "xlink:href",
  "action",
  "formAction",
  "formaction",
  "poster",
  "data",
  "srcSet",
  "srcset",
]);

function rehypeSanitizeRenderedHtml(filePath?: string | null) {
  return (tree: HastNode) => {
    sanitizeHastChildren(tree, filePath);
  };
}

function sanitizeHastChildren(parent: HastNode, filePath?: string | null): void {
  const children = parent.children;
  if (!children) return;

  for (let index = children.length - 1; index >= 0; index -= 1) {
    const child = children[index];
    if (child.type === "comment") {
      children.splice(index, 1);
      continue;
    }

    if (child.type === "element") {
      const tagName = child.tagName?.toLowerCase() ?? "";
      if (blockedHtmlTags.has(tagName)) {
        children.splice(index, 1);
        continue;
      }
      sanitizeHastElement(child, filePath);
    }

    sanitizeHastChildren(child, filePath);
  }
}

function sanitizeHastElement(node: HastNode, filePath?: string | null): void {
  const properties = node.properties;
  if (!properties) return;

  for (const [name, value] of Object.entries(properties)) {
    const normalized = name.toLowerCase();
    if (normalized.startsWith("on")) {
      delete properties[name];
      continue;
    }

    if (normalized === "style") {
      const cleanStyle = sanitizeInlineStyle(String(value));
      if (cleanStyle) {
        properties[name] = cleanStyle;
      } else {
        delete properties[name];
      }
      continue;
    }

    if (urlAttributeNames.has(name) || urlAttributeNames.has(normalized)) {
      if (normalized === "srcset") {
        const rewritten = rewriteSrcsetValue(String(value), filePath);
        if (rewritten) {
          properties[name] = rewritten;
        } else {
          delete properties[name];
        }
      } else {
        const rewritten = rewriteSafeUrlValue(String(value), filePath);
        if (rewritten) {
          properties[name] = rewritten;
        } else {
          delete properties[name];
        }
      }
    }
  }
}

function sanitizeInlineStyle(style: string): string {
  const clean = style
    .split(";")
    .map((declaration) => declaration.trim())
    .filter((declaration) => {
      if (!declaration) return false;
      const lower = declaration.toLowerCase();
      return !(
        lower.includes("expression(") ||
        lower.includes("javascript:") ||
        lower.includes("vbscript:") ||
        lower.includes("@import") ||
        /url\s*\(/i.test(declaration)
      );
    })
    .join("; ");

  return clean ? `${clean};` : "";
}

function isSafeUrlValue(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed || trimmed.startsWith("#")) return true;

  const lower = trimmed.toLowerCase();
  if (
    lower.startsWith("javascript:") ||
    lower.startsWith("vbscript:") ||
    lower.startsWith("data:text/javascript")
  ) {
    return false;
  }

  if (lower.startsWith("data:")) {
    return lower.startsWith("data:image/") || lower.startsWith("data:application/pdf");
  }

  if (
    !trimmed.includes(":") ||
    trimmed.startsWith("./") ||
    trimmed.startsWith("../") ||
    trimmed.startsWith("/")
  ) {
    return true;
  }

  try {
    const parsed = new URL(trimmed, document.baseURI);
    return ["http:", "https:", "mailto:", "tel:", "file:", "asset:", "blob:"].includes(
      parsed.protocol,
    );
  } catch {
    return true;
  }
}

function rewriteSafeUrlValue(value: string, filePath?: string | null): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const localPath = resolveLocalMarkdownPath(trimmed, filePath);
  if (localPath) return convertFileSrc(localPath);
  return isSafeUrlValue(trimmed) ? trimmed : null;
}

function rewriteSrcsetValue(value: string, filePath?: string | null): string | null {
  const candidates = value
    .split(",")
    .map((candidate) => candidate.trim())
    .filter(Boolean);
  if (candidates.length === 0) return null;

  const rewritten: string[] = [];
  for (const candidate of candidates) {
    const [urlPart, ...rest] = candidate.split(/\s+/);
    const safeUrl = rewriteSafeUrlValue(urlPart, filePath);
    if (!safeUrl) return null;
    rewritten.push(rest.length > 0 ? `${safeUrl} ${rest.join(" ")}` : safeUrl);
  }
  return rewritten.join(", ");
}

export default function PlainMarkdownPreview({
  text,
  filePath = null,
}: {
  text: string;
  filePath?: string | null;
}) {
  const themeType = useEffectiveThemeType();
  const components = useMemo(
    () => createMarkdownComponents(themeType, filePath),
    [filePath, themeType],
  );

  return (
    <ScrollArea
      className="sessio-plain-editor-preview min-h-0 flex-1"
      viewportClassName="sessio-plain-editor-preview-viewport"
      persistScrollbars
    >
      <article
        className="sessio-plain-editor-preview-content markdown-content"
        data-theme-type={themeType}
      >
        <PlainMarkdownPreviewContent
          text={text}
          components={components}
          filePath={filePath}
          themeType={themeType}
        />
      </article>
    </ScrollArea>
  );
}

export function PlainMarkdownPreviewContent({
  text,
  components,
  filePath = null,
  themeType = "light",
}: {
  text: string;
  components?: Components;
  filePath?: string | null;
  themeType?: PreviewThemeType;
}) {
  const normalizedText = useMemo(() => normalizePreviewMarkdown(text), [text]);
  const resolvedComponents = useMemo(
    () => components ?? createMarkdownComponents(themeType, filePath),
    [components, filePath, themeType],
  );

  return (
    <ReactMarkdown
      remarkPlugins={[[remarkGfm, { singleTilde: false }], remarkBreaks, remarkMath, remarkSuperSub]}
      rehypePlugins={[rehypeRaw, [rehypeSanitizeRenderedHtml, filePath], rehypeKatex]}
      components={resolvedComponents}
      urlTransform={(url) => markdownUrlTransform(url, filePath)}
    >
      {normalizedText}
    </ReactMarkdown>
  );
}

type MdastNode = {
  type: string;
  value?: string;
  children?: MdastNode[];
  data?: Record<string, unknown>;
};

const scriptSyntaxPattern =
  /(?<!\^)\^([^\s^][^^]*?[^\s^]|[^\s^])\^(?!\^)|(?<!~)~([^\s~][^~]*?[^\s~]|[^\s~])~(?!~)/g;

function remarkSuperSub() {
  return (tree: MdastNode) => {
    transformScriptSyntax(tree);
  };
}

function transformScriptSyntax(parent: MdastNode): void {
  const children = parent.children;
  if (!children) return;

  for (let index = children.length - 1; index >= 0; index -= 1) {
    const child = children[index];
    if (child.type === "text" && child.value) {
      if (!child.value.includes("^") && !child.value.includes("~")) continue;

      const parsed = parseScriptSyntax(child.value);
      if (parsed.length === 1 && parsed[0]?.type === "text") continue;
      children.splice(index, 1, ...parsed);
      continue;
    }

    transformScriptSyntax(child);
  }
}

function parseScriptSyntax(text: string): MdastNode[] {
  const result: MdastNode[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  scriptSyntaxPattern.lastIndex = 0;
  while ((match = scriptSyntaxPattern.exec(text)) !== null) {
    if (match.index > lastIndex) {
      result.push({ type: "text", value: text.slice(lastIndex, match.index) });
    }

    const isSuperscript = match[0].startsWith("^");
    const content = isSuperscript ? match[1] : match[2];
    result.push({
      type: isSuperscript ? "superscript" : "subscript",
      children: [{ type: "text", value: content }],
      data: { hName: isSuperscript ? "sup" : "sub" },
    });

    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < text.length) {
    result.push({ type: "text", value: text.slice(lastIndex) });
  }

  return result;
}

function normalizePreviewMarkdown(text: string): string {
  return coalesceBrokenMarkdownImageLinks(text);
}

function coalesceBrokenMarkdownImageLinks(text: string): string {
  const lines = text.split(/\r?\n/);
  const merged: string[] = [];

  for (let index = 0; index < lines.length; index += 1) {
    const current = lines[index] ?? "";
    const next = lines[index + 1] ?? "";
    const currentMatch = current.match(/^(\s*)!\[([^\]\n]*)]\s*$/);
    const nextMatch = next.match(
      /^\s*\(((?:asset:\/\/|file:\/\/|https?:\/\/|\/|\.\/|\.\.\/)[^)\n]*)\)\s*$/,
    );

    if (currentMatch && nextMatch) {
      merged.push(`${currentMatch[1]}![${currentMatch[2]}](${nextMatch[1]})`);
      index += 1;
      continue;
    }

    merged.push(current);
  }

  return merged.join("\n");
}

function createMarkdownComponents(
  themeType: PreviewThemeType = "light",
  filePath?: string | null,
): Components {
  const themedElement = createThemedPreviewElement(themeType);

  return {
    p: themedElement("p"),
    blockquote: themedElement("blockquote"),
    div: ({ node: _node, children, style, ...props }) => (
      <PlainPreviewDiv style={style} themeType={themeType} {...props}>
        {children}
      </PlainPreviewDiv>
    ),
    span: themedElement("span"),
    section: themedElement("section"),
    article: themedElement("article"),
    h1: themedElement("h1"),
    h2: themedElement("h2"),
    h3: themedElement("h3"),
    h4: themedElement("h4"),
    h5: themedElement("h5"),
    h6: themedElement("h6"),
    table: themedElement("table"),
    thead: themedElement("thead"),
    tbody: themedElement("tbody"),
    tr: themedElement("tr"),
    th: themedElement("th"),
    td: themedElement("td"),
    ul: themedElement("ul"),
    ol: themedElement("ol"),
    li: themedElement("li"),
    strong: themedElement("strong"),
    em: themedElement("em"),
    style: ({ node: _node, children, ...props }) => (
      <PlainPreviewStyle themeType={themeType} {...props}>
        {children}
      </PlainPreviewStyle>
    ),
    input: ({ type, checked, disabled }) => renderMarkdownInput({ type, checked, disabled }),
    pre: ({ children }) => <>{children}</>,
    code: ({ children, className }) => {
      if (className) {
        return (
          <PlainPreviewCodeBlock
            code={codeTextFromChildren(children)}
            language={codeLanguageFromClassName(className)}
            themeType={themeType}
          />
        );
      }
      return <code>{children}</code>;
    },
    a: ({ children, href }) => {
      const safe = safeHref(href ?? "", filePath);
      if (!safe) return <>{children}</>;
      return (
        <a href={safe} target="_blank" rel="noreferrer">
          {children}
        </a>
      );
    },
    img: ({ src, alt }) => {
      if (!isRenderableMarkdownImageSrc(src ?? "", filePath)) {
        return <code>{`![${alt ?? "image"}](${src ?? ""})`}</code>;
      }
      return <PlainPreviewImage src={src ?? ""} alt={alt ?? "image"} filePath={filePath} />;
    },
  };
}

function PlainPreviewImage({
  src,
  alt,
  filePath,
}: {
  src: string;
  alt: string;
  filePath?: string | null;
}) {
  const resolvedSrc = useResolvedImageSrc(src, filePath);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [resolvedSrc]);

  if (!resolvedSrc || failed) {
    return <code>{`![${alt}](${src})`}</code>;
  }

  return <img src={resolvedSrc} alt={alt} loading="lazy" onError={() => setFailed(true)} />;
}

function createThemedPreviewElement(themeType: PreviewThemeType) {
  return function themedElement(tagName: keyof HTMLElementTagNameMap) {
    return function ThemedPreviewElement({
      node: _node,
      children,
      style,
      ...props
    }: HTMLAttributes<HTMLElement> & {
      children?: ReactNode;
      node?: unknown;
      style?: CSSProperties;
    }) {
      return createElement(
        tagName,
        {
          ...props,
          style: adaptPreviewInlineStyle(style, themeType),
        },
        children,
      );
    };
  };
}

function PlainPreviewDiv({
  children,
  style,
  themeType,
  ...props
}: HTMLAttributes<HTMLDivElement> & { node?: unknown; themeType: PreviewThemeType }) {
  const adaptedStyle = adaptPreviewInlineStyle(style, themeType);

  if (hasFixedPixelWidth(style)) {
    return (
      <PlainPreviewScaledHtmlBlock style={adaptedStyle} {...props}>
        {children}
      </PlainPreviewScaledHtmlBlock>
    );
  }

  return (
    <div style={adaptedStyle} {...props}>
      {children}
    </div>
  );
}

function PlainPreviewStyle({
  children,
  themeType,
  ...props
}: StyleHTMLAttributes<HTMLStyleElement> & {
  children?: ReactNode;
  node?: unknown;
  themeType: PreviewThemeType;
}) {
  const css = adaptPreviewStyleSheet(codeTextFromChildren(children), themeType);
  if (!css.trim()) return null;

  return <style {...props}>{css}</style>;
}

function PlainPreviewScaledHtmlBlock({
  children,
  style,
  ...props
}: HTMLAttributes<HTMLDivElement>) {
  const frameRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [layout, setLayout] = useState({
    height: undefined as number | undefined,
    marginLeft: 0,
    scale: 1,
    width: undefined as number | undefined,
  });

  const measure = useCallback(() => {
    const frame = frameRef.current;
    const content = contentRef.current;
    if (!frame || !content) return;

    const article = frame.closest(".sessio-plain-editor-preview-content") as HTMLElement | null;
    const viewport = frame.closest(".sessio-plain-editor-preview-viewport") as HTMLElement | null;
    const articleWidth = article?.clientWidth || frame.parentElement?.clientWidth || frame.clientWidth;
    const fixedWidth = cssPixelValue(style?.width) ?? cssPixelValue(style?.minWidth) ?? 0;
    const naturalWidth = Math.max(content.scrollWidth, content.offsetWidth, fixedWidth);

    let nextWidth = articleWidth;
    let nextMarginLeft = 0;
    if (viewport && naturalWidth > articleWidth + 1) {
      const viewportStyle = window.getComputedStyle(viewport);
      const paddingLeft = Number.parseFloat(viewportStyle.paddingLeft) || 0;
      const paddingRight = Number.parseFloat(viewportStyle.paddingRight) || 0;
      const viewportRect = viewport.getBoundingClientRect();
      const frameRect = frame.getBoundingClientRect();
      nextWidth = Math.max(0, viewport.clientWidth - paddingLeft - paddingRight);
      nextMarginLeft = viewportRect.left + paddingLeft - frameRect.left;
    }

    const nextScale = naturalWidth > nextWidth && nextWidth > 0 ? nextWidth / naturalWidth : 1;
    const naturalHeight = Math.max(content.scrollHeight, content.offsetHeight);
    const nextHeight =
      nextScale < 1 && naturalHeight > 0 ? Math.ceil(naturalHeight * nextScale) : undefined;

    setLayout((current) => {
      if (
        Math.abs(current.scale - nextScale) < 0.001 &&
        Math.abs((current.height ?? 0) - (nextHeight ?? 0)) < 1 &&
        Math.abs(current.marginLeft - nextMarginLeft) < 1 &&
        Math.abs((current.width ?? 0) - nextWidth) < 1
      ) {
        return current;
      }

      return {
        height: nextHeight,
        marginLeft: nextMarginLeft,
        scale: nextScale,
        width: nextWidth,
      };
    });
  }, [style?.minWidth, style?.width]);

  useEffect(() => {
    measure();

    const frame = frameRef.current;
    const content = contentRef.current;
    const viewport = frame?.closest(".sessio-plain-editor-preview-viewport");
    if (!frame || !content || typeof ResizeObserver === "undefined") return;

    const observer = new ResizeObserver(() => measure());
    observer.observe(frame);
    observer.observe(content);
    if (viewport instanceof Element) observer.observe(viewport);

    return () => observer.disconnect();
  }, [measure]);

  return (
    <div
      ref={frameRef}
      className="sessio-plain-editor-html-fit"
      style={{
        height: layout.height,
        marginLeft: layout.marginLeft,
        width: layout.width,
      }}
    >
      <div
        ref={contentRef}
        className="sessio-plain-editor-html-fit-inner"
        style={{
          ...style,
          transform: layout.scale < 1 ? `scale(${layout.scale})` : undefined,
        }}
        {...props}
      >
        {children}
      </div>
    </div>
  );
}

function hasFixedPixelWidth(style: CSSProperties | undefined): boolean {
  return cssPixelValue(style?.width) !== null || cssPixelValue(style?.minWidth) !== null;
}

function cssPixelValue(value: CSSProperties["width"] | CSSProperties["minWidth"]): number | null {
  if (typeof value === "number") return value > 0 ? value : null;
  if (typeof value !== "string") return null;

  const match = value.trim().match(/^(\d+(?:\.\d+)?)px$/i);
  return match ? Number.parseFloat(match[1]) : null;
}

type PreviewColorRole = "background" | "border" | "text";

type ParsedCssColor = {
  a: number;
  b: number;
  g: number;
  h: number;
  l: number;
  luminance: number;
  r: number;
  s: number;
};

const cssColorPattern =
  /#[0-9a-fA-F]{3,8}\b|rgba?\([^)]+\)|hsla?\([^)]+\)|\b(?:black|white|lightblue|lightgreen|lightcoral|lightgrey|lightgray|darkblue|steelblue|firebrick)\b/g;

const namedCssColors: Record<string, [number, number, number]> = {
  black: [0, 0, 0],
  darkblue: [0, 0, 139],
  firebrick: [178, 34, 34],
  lightblue: [173, 216, 230],
  lightcoral: [240, 128, 128],
  lightgray: [211, 211, 211],
  lightgreen: [144, 238, 144],
  lightgrey: [211, 211, 211],
  steelblue: [70, 130, 180],
  white: [255, 255, 255],
};

const previewStyleTextProps = new Set([
  "color",
  "caretColor",
  "textDecorationColor",
  "WebkitTextFillColor",
]);

const previewStyleBackgroundProps = new Set([
  "background",
  "backgroundColor",
]);

const previewStyleBorderProps = new Set([
  "border",
  "borderBlock",
  "borderBlockColor",
  "borderBlockEnd",
  "borderBlockEndColor",
  "borderBlockStart",
  "borderBlockStartColor",
  "borderBottom",
  "borderBottomColor",
  "borderColor",
  "borderInline",
  "borderInlineColor",
  "borderInlineEnd",
  "borderInlineEndColor",
  "borderInlineStart",
  "borderInlineStartColor",
  "borderLeft",
  "borderLeftColor",
  "borderRight",
  "borderRightColor",
  "borderTop",
  "borderTopColor",
  "columnRule",
  "columnRuleColor",
  "outline",
  "outlineColor",
]);

function adaptPreviewInlineStyle(
  style: CSSProperties | undefined,
  themeType: PreviewThemeType,
): CSSProperties | undefined {
  if (themeType !== "dark" || !style) return style;

  let changed = false;
  const next = { ...style } as Record<string, unknown>;
  for (const [property, value] of Object.entries(next)) {
    if (typeof value !== "string") continue;

    const role = previewStyleRole(property);
    if (!role) continue;

    const adapted = adaptCssColorValue(value, role);
    if (adapted !== value) {
      next[property] = adapted;
      changed = true;
    }
  }

  return changed ? (next as CSSProperties) : style;
}

function adaptPreviewStyleSheet(css: string, themeType: PreviewThemeType): string {
  const cleanCss = sanitizeStyleSheet(css);
  if (themeType !== "dark") return cleanCss;

  const overrides = createDarkStyleSheetOverrides(cleanCss);
  return overrides ? `${cleanCss}\n${overrides}` : cleanCss;
}

function sanitizeStyleSheet(css: string): string {
  const lower = css.toLowerCase();
  if (
    lower.includes("@import") ||
    lower.includes("expression(") ||
    lower.includes("javascript:") ||
    lower.includes("vbscript:") ||
    /url\s*\(/i.test(css)
  ) {
    return "";
  }

  return css;
}

function createDarkStyleSheetOverrides(css: string): string {
  const overrides: string[] = [];
  const rulePattern = /([^{}]+)\{([^{}]+)\}/g;
  let match: RegExpExecArray | null;

  while ((match = rulePattern.exec(css)) !== null) {
    const selector = match[1]?.trim() ?? "";
    const declarations = match[2] ?? "";
    if (!selector || selector.startsWith("@")) continue;

    const adaptedDeclarations = adaptCssDeclarationBlock(declarations);
    if (!adaptedDeclarations || adaptedDeclarations === declarations.trim()) continue;

    const scopedSelector = selector
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean)
      .map((item) => `.sessio-plain-editor-preview-content[data-theme-type="dark"] ${item}`)
      .join(", ");
    if (!scopedSelector) continue;

    overrides.push(`${scopedSelector} { ${adaptedDeclarations} }`);
  }

  return overrides.join("\n");
}

function adaptCssDeclarationBlock(declarations: string): string {
  let changed = false;
  const adapted = declarations
    .split(";")
    .map((declaration) => {
      const trimmed = declaration.trim();
      if (!trimmed) return "";

      const separator = trimmed.indexOf(":");
      if (separator < 0) return trimmed;

      const property = trimmed.slice(0, separator).trim();
      const value = trimmed.slice(separator + 1).trim();
      const role = previewCssPropertyRole(property);
      if (!role) return trimmed;

      const nextValue = adaptCssColorValue(value, role);
      if (nextValue !== value) changed = true;
      return `${property}: ${nextValue}`;
    })
    .filter(Boolean)
    .join("; ");

  return changed ? `${adapted};` : declarations.trim();
}

function previewStyleRole(property: string): PreviewColorRole | null {
  if (previewStyleTextProps.has(property)) return "text";
  if (previewStyleBackgroundProps.has(property)) return "background";
  if (previewStyleBorderProps.has(property)) return "border";
  if (property === "fill") return "background";
  if (property === "stroke") return "border";
  return null;
}

function previewCssPropertyRole(property: string): PreviewColorRole | null {
  const normalized = property.trim().toLowerCase();
  if (!normalized) return null;
  if (normalized === "color" || normalized.endsWith("-color") && normalized.includes("text")) {
    return "text";
  }
  if (normalized.includes("background")) return "background";
  if (
    normalized.includes("border") ||
    normalized.includes("outline") ||
    normalized === "stroke" ||
    normalized === "column-rule"
  ) {
    return "border";
  }
  if (normalized === "fill") return "background";
  if (normalized === "color") return "text";
  return null;
}

function adaptCssColorValue(value: string, role: PreviewColorRole): string {
  return value.replace(cssColorPattern, (token) => adaptCssColorToken(token, role));
}

function adaptCssColorToken(token: string, role: PreviewColorRole): string {
  const color = parseCssColor(token);
  if (!color || color.a <= 0.001) return token;

  if (role === "background") {
    return adaptBackgroundColor(token, color);
  }

  if (role === "border") {
    return adaptBorderColor(token, color);
  }

  return adaptTextColor(token, color);
}

function adaptBackgroundColor(token: string, color: ParsedCssColor): string {
  if (color.a < 0.18) return token;
  if (color.luminance <= 0.42) return token;

  if (color.s < 0.08) {
    return color.a < 1 ? "rgb(var(--color-fg) / 0.08)" : "rgb(var(--color-fg) / 0.055)";
  }

  const lightness = color.luminance > 0.72 ? 18 : 24;
  const saturation = clamp(color.s * 65, 22, 48);
  const alpha = color.a < 1 ? Math.min(0.24, Math.max(0.1, color.a * 0.18)) : 0.86;
  return hslColor(color.h, saturation, lightness, alpha);
}

function adaptBorderColor(token: string, color: ParsedCssColor): string {
  if (color.a < 0.18) return token;

  if (color.s < 0.08) {
    return "rgb(var(--color-fg) / 0.16)";
  }

  if (color.luminance > 0.56 || color.luminance < 0.28) {
    return hslColor(color.h, clamp(color.s * 70, 32, 58), 52, Math.min(0.72, color.a));
  }

  return token;
}

function adaptTextColor(token: string, color: ParsedCssColor): string {
  if (color.a < 0.18) return token;
  if (color.luminance >= 0.52) return token;

  if (color.s < 0.14) {
    return color.luminance < 0.28
      ? "var(--plain-editor-heading)"
      : "var(--plain-editor-text)";
  }

  return hslColor(color.h, clamp(color.s * 82, 42, 74), 72, Math.min(1, color.a));
}

function parseCssColor(token: string): ParsedCssColor | null {
  const normalized = token.trim().toLowerCase();
  if (!normalized || normalized === "transparent" || normalized === "currentcolor") return null;

  if (normalized.startsWith("#")) return parseHexColor(normalized);
  if (normalized.startsWith("rgb")) return parseRgbColor(normalized);
  if (normalized.startsWith("hsl")) return parseHslColor(normalized);

  const named = namedCssColors[normalized];
  return named ? parsedColor(named[0], named[1], named[2], 1) : null;
}

function parseHexColor(value: string): ParsedCssColor | null {
  const hex = value.slice(1);
  if (![3, 4, 6, 8].includes(hex.length)) return null;

  const expanded = hex.length <= 4
    ? hex.split("").map((item) => item + item).join("")
    : hex;
  const r = Number.parseInt(expanded.slice(0, 2), 16);
  const g = Number.parseInt(expanded.slice(2, 4), 16);
  const b = Number.parseInt(expanded.slice(4, 6), 16);
  const a = expanded.length === 8 ? Number.parseInt(expanded.slice(6, 8), 16) / 255 : 1;
  if ([r, g, b, a].some((item) => Number.isNaN(item))) return null;

  return parsedColor(r, g, b, a);
}

function parseRgbColor(value: string): ParsedCssColor | null {
  const body = value.slice(value.indexOf("(") + 1, -1).trim();
  const [channelPart, slashAlpha] = body.split("/").map((part) => part.trim());
  const parts = channelPart.includes(",")
    ? channelPart.split(",").map((part) => part.trim())
    : channelPart.split(/\s+/).filter(Boolean);
  if (parts.length < 3) return null;

  const alphaPart = slashAlpha ?? (parts.length > 3 ? parts[3] : undefined);
  const r = parseRgbChannel(parts[0]);
  const g = parseRgbChannel(parts[1]);
  const b = parseRgbChannel(parts[2]);
  const a = parseAlpha(alphaPart);
  if ([r, g, b, a].some((item) => Number.isNaN(item))) return null;

  return parsedColor(r, g, b, a);
}

function parseHslColor(value: string): ParsedCssColor | null {
  const body = value.slice(value.indexOf("(") + 1, -1).trim();
  const [channelPart, slashAlpha] = body.split("/").map((part) => part.trim());
  const parts = channelPart.includes(",")
    ? channelPart.split(",").map((part) => part.trim())
    : channelPart.split(/\s+/).filter(Boolean);
  if (parts.length < 3) return null;

  const alphaPart = slashAlpha ?? (parts.length > 3 ? parts[3] : undefined);
  const h = parseHue(parts[0]);
  const s = parsePercent(parts[1]);
  const l = parsePercent(parts[2]);
  const a = parseAlpha(alphaPart);
  if ([h, s, l, a].some((item) => Number.isNaN(item))) return null;

  const { r, g, b } = hslToRgb(h, s / 100, l / 100);
  return parsedColor(r, g, b, a);
}

function parsedColor(r: number, g: number, b: number, a: number): ParsedCssColor {
  const hsl = rgbToHsl(r, g, b);
  return {
    a,
    b,
    g,
    h: hsl.h,
    l: hsl.l,
    luminance: relativeLuminance(r, g, b),
    r,
    s: hsl.s,
  };
}

function parseRgbChannel(value: string): number {
  if (value.endsWith("%")) return clamp((Number.parseFloat(value) / 100) * 255, 0, 255);
  return clamp(Number.parseFloat(value), 0, 255);
}

function parseAlpha(value: string | undefined): number {
  if (!value) return 1;
  if (value.endsWith("%")) return clamp(Number.parseFloat(value) / 100, 0, 1);
  return clamp(Number.parseFloat(value), 0, 1);
}

function parseHue(value: string): number {
  const amount = Number.parseFloat(value);
  if (Number.isNaN(amount)) return Number.NaN;
  if (value.endsWith("turn")) return normalizeHue(amount * 360);
  if (value.endsWith("rad")) return normalizeHue((amount * 180) / Math.PI);
  if (value.endsWith("grad")) return normalizeHue(amount * 0.9);
  return normalizeHue(amount);
}

function parsePercent(value: string): number {
  return clamp(Number.parseFloat(value), 0, 100);
}

function relativeLuminance(r: number, g: number, b: number): number {
  const [rs, gs, bs] = [r, g, b].map((channel) => {
    const normalized = channel / 255;
    return normalized <= 0.03928
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * rs + 0.7152 * gs + 0.0722 * bs;
}

function rgbToHsl(r: number, g: number, b: number): { h: number; l: number; s: number } {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const l = (max + min) / 2;

  if (max === min) return { h: 0, l, s: 0 };

  const delta = max - min;
  const s = l > 0.5 ? delta / (2 - max - min) : delta / (max + min);
  let h = 0;
  if (max === rn) h = (gn - bn) / delta + (gn < bn ? 6 : 0);
  else if (max === gn) h = (bn - rn) / delta + 2;
  else h = (rn - gn) / delta + 4;

  return { h: normalizeHue(h * 60), l, s };
}

function hslToRgb(h: number, s: number, l: number): { b: number; g: number; r: number } {
  const hue = normalizeHue(h) / 360;
  if (s === 0) {
    const gray = l * 255;
    return { b: gray, g: gray, r: gray };
  }

  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const r = hueToRgb(p, q, hue + 1 / 3) * 255;
  const g = hueToRgb(p, q, hue) * 255;
  const b = hueToRgb(p, q, hue - 1 / 3) * 255;
  return { b, g, r };
}

function hueToRgb(p: number, q: number, t: number): number {
  let next = t;
  if (next < 0) next += 1;
  if (next > 1) next -= 1;
  if (next < 1 / 6) return p + (q - p) * 6 * next;
  if (next < 1 / 2) return q;
  if (next < 2 / 3) return p + (q - p) * (2 / 3 - next) * 6;
  return p;
}

function hslColor(h: number, s: number, l: number, a = 1): string {
  const alpha = Math.round(a * 100) / 100;
  return alpha >= 0.99
    ? `hsl(${Math.round(normalizeHue(h))} ${Math.round(s)}% ${Math.round(l)}%)`
    : `hsl(${Math.round(normalizeHue(h))} ${Math.round(s)}% ${Math.round(l)}% / ${alpha})`;
}

function normalizeHue(value: number): number {
  return ((value % 360) + 360) % 360;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function PlainPreviewCodeBlock({
  code,
  language,
  themeType,
}: {
  code: string;
  language: string;
  themeType: PreviewThemeType;
}) {
  const diagramKind = diagramKindFromLanguage(language);
  if (diagramKind) {
    return <PlainPreviewDiagramBlock code={code} kind={diagramKind} themeType={themeType} />;
  }

  return <PlainPreviewCodeBlockInner code={code} language={language} themeType={themeType} />;
}

function PlainPreviewCodeBlockInner({
  code,
  language,
  themeType,
}: {
  code: string;
  language: string;
  themeType: PreviewThemeType;
}) {
  const highlighted = useShikiHighlightedCode(code, language, themeType);

  return (
    <ScrollArea
      className="sessio-plain-editor-preview-code"
      viewportClassName="px-3 py-2"
      orientation="horizontal"
      persistScrollbars
    >
      <pre>
        <code>{highlighted ?? code}</code>
      </pre>
    </ScrollArea>
  );
}

type PreviewDiagramKind = "mermaid" | "dot" | "vega" | "vega-lite" | "infographic";

function PlainPreviewDiagramBlock({
  code,
  kind,
  themeType,
}: {
  code: string;
  kind: PreviewDiagramKind;
  themeType: PreviewThemeType;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [rendering, setRendering] = useState(true);

  useEffect(() => {
    let cancelled = false;
    let cleanup: (() => void) | undefined;
    const host = hostRef.current;
    if (!host) return;

    setError(null);
    setRendering(true);
    host.innerHTML = "";

    renderDiagramBlock(kind, code, host, themeType)
      .then((nextCleanup) => {
        if (cancelled) {
          nextCleanup?.();
          return;
        }
        cleanup = nextCleanup;
        setRendering(false);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        host.innerHTML = "";
        setError(err instanceof Error ? err.message : String(err));
        setRendering(false);
      });

    return () => {
      cancelled = true;
      cleanup?.();
      host.innerHTML = "";
    };
  }, [code, kind, themeType]);

  return (
    <div
      className={error ? "sessio-plain-editor-diagram-error" : "sessio-plain-editor-diagram"}
      data-diagram-kind={kind}
    >
      {error ? (
        <>
          <div className="mb-2 text-caption text-status-error">{error}</div>
          <PlainPreviewCodeBlockInner code={code} language={kind} themeType={themeType} />
        </>
      ) : rendering ? (
        <div className="sessio-plain-editor-diagram-loading">
          Rendering {diagramLabel(kind)}...
        </div>
      ) : null}
      <div
        ref={hostRef}
        className={error ? "hidden" : "sessio-plain-editor-diagram-host"}
      />
    </div>
  );
}

async function renderDiagramBlock(
  kind: PreviewDiagramKind,
  code: string,
  host: HTMLElement,
  themeType: PreviewThemeType,
): Promise<(() => void) | undefined> {
  switch (kind) {
    case "mermaid":
      await renderMermaidDiagram(code, host, themeType);
      return undefined;
    case "dot":
      await renderDotDiagram(code, host, themeType);
      return undefined;
    case "vega":
    case "vega-lite":
      return renderVegaDiagram(code, kind, host, themeType);
    case "infographic":
      return renderInfographicDiagram(code, host, themeType);
  }
}

async function renderMermaidDiagram(
  code: string,
  host: HTMLElement,
  themeType: PreviewThemeType,
): Promise<void> {
  const mermaid = (await import("mermaid")).default;
  mermaid.initialize({
    startOnLoad: false,
    securityLevel: "strict",
    theme: themeType === "light" ? "default" : "dark",
  });
  const id = `sessio-mermaid-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const { svg } = await mermaid.render(id, code);
  host.innerHTML = svg;
}

async function renderDotDiagram(
  code: string,
  host: HTMLElement,
  themeType: PreviewThemeType,
): Promise<void> {
  const { instance } = await import("@viz-js/viz");
  const viz = await instance();
  const svg = viz.renderSVGElement(code, {
    graphAttributes: dotGraphAttributes(themeType),
    nodeAttributes: dotNodeAttributes(themeType),
    edgeAttributes: dotEdgeAttributes(themeType),
  });
  adaptSvgElementTheme(svg, themeType);
  host.replaceChildren(svg);
}

async function renderVegaDiagram(
  code: string,
  kind: "vega" | "vega-lite",
  host: HTMLElement,
  themeType: PreviewThemeType,
): Promise<() => void> {
  const [{ default: embed }, { expressionInterpreter }] = await Promise.all([
    import("vega-embed"),
    import("vega-interpreter"),
  ]);
  const spec = JSON.parse(code) as VisualizationSpec;
  const result = await embed(host, spec, {
    mode: kind === "vega" ? "vega" : "vega-lite",
    actions: false,
    renderer: "svg",
    ast: true,
    expr: expressionInterpreter,
    config: vegaThemeConfig(themeType),
  });
  await result.view?.runAsync?.();
  return () => {
    result.view?.finalize?.();
  };
}

async function renderInfographicDiagram(
  code: string,
  host: HTMLElement,
  themeType: PreviewThemeType,
): Promise<() => void> {
  const { Infographic, setDefaultFont } = await import("@antv/infographic");
  setDefaultFont(
    "Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif",
  );
  const infographic = new Infographic({
    container: host,
    width: 900,
    height: 600,
    padding: 24,
    ...(themeType === "dark"
      ? {
          themeConfig: {
            colorBg: "#2b313b",
            colorPrimary: "#45c9b9",
            base: {
              text: {
                fill: "#dce4ee",
              },
            },
          },
          svg: {
            background: false,
          },
        }
      : {
          svg: {
            background: false,
          },
        }),
  });
  await new Promise<void>((resolve, reject) => {
    const timeout = window.setTimeout(() => {
      reject(new Error("Infographic render timeout after 10s"));
    }, 10000);
    infographic.on("rendered", () => {
      window.clearTimeout(timeout);
      resolve();
    });
    infographic.on("error", (err: unknown) => {
      window.clearTimeout(timeout);
      reject(err instanceof Error ? err : new Error(String(err)));
    });
    try {
      infographic.render(code);
    } catch (err) {
      window.clearTimeout(timeout);
      reject(err instanceof Error ? err : new Error(String(err)));
    }
  });
  return () => {
    infographic.destroy();
  };
}

function diagramKindFromLanguage(language: string): PreviewDiagramKind | null {
  const normalized = language.toLowerCase().trim();
  if (normalized === "mermaid" || normalized === "mmd") return "mermaid";
  if (normalized === "dot" || normalized === "graphviz" || normalized === "gv") return "dot";
  if (normalized === "vega") return "vega";
  if (normalized === "vega-lite" || normalized === "vegalite" || normalized === "vl") return "vega-lite";
  if (normalized === "infographic") return "infographic";
  return null;
}

function diagramLabel(kind: PreviewDiagramKind): string {
  if (kind === "dot") return "Graphviz";
  if (kind === "vega-lite") return "Vega-Lite";
  return kind.charAt(0).toUpperCase() + kind.slice(1);
}

function dotGraphAttributes(themeType: PreviewThemeType): Record<string, string> {
  if (themeType === "light") {
    return {
      bgcolor: "transparent",
    };
  }

  return {
    bgcolor: "transparent",
    color: "#5f6b7a",
    fontcolor: "#dce4ee",
  };
}

function dotNodeAttributes(themeType: PreviewThemeType): Record<string, string> | undefined {
  if (themeType === "light") return undefined;

  return {
    color: "#5f6b7a",
    fillcolor: "#313946",
    fontcolor: "#dce4ee",
  };
}

function dotEdgeAttributes(themeType: PreviewThemeType): Record<string, string> | undefined {
  if (themeType === "light") return undefined;

  return {
    color: "#7d8794",
    fontcolor: "#c4cfdd",
  };
}

function vegaThemeConfig(themeType: PreviewThemeType) {
  if (themeType === "light") {
    return {
      background: null,
      view: {
        stroke: null,
      },
    };
  }

  const text = "#dce4ee";
  const muted = "#a8b4c4";
  const grid = "#5f6b7a";
  return {
    axis: {
      domainColor: grid,
      gridColor: "rgba(218, 224, 234, 0.14)",
      labelColor: muted,
      tickColor: grid,
      titleColor: text,
    },
    background: null,
    header: {
      labelColor: muted,
      titleColor: text,
    },
    legend: {
      labelColor: muted,
      titleColor: text,
    },
    title: {
      color: text,
      subtitleColor: muted,
    },
    view: {
      stroke: null,
    },
  };
}

function adaptSvgElementTheme(svg: SVGSVGElement, themeType: PreviewThemeType): void {
  if (themeType !== "dark") return;

  const elements = [svg, ...Array.from(svg.querySelectorAll<SVGElement>("*"))];
  for (const element of elements) {
    for (const attribute of ["fill", "stroke", "color"]) {
      const value = element.getAttribute(attribute);
      if (!value || value.startsWith("url(") || value === "none" || value === "transparent") {
        continue;
      }

      const role: PreviewColorRole = attribute === "stroke" ? "border" : "background";
      const nextValue = adaptCssColorValue(value, role);
      if (nextValue !== value) element.setAttribute(attribute, nextValue);
    }

    const style = element.getAttribute("style");
    if (style) {
      const adaptedStyle = adaptCssDeclarationBlock(style);
      if (adaptedStyle !== style.trim()) element.setAttribute("style", adaptedStyle);
    }
  }

  for (const text of Array.from(svg.querySelectorAll("text"))) {
    const fill = text.getAttribute("fill");
    const parsed = fill ? parseCssColor(fill) : null;
    if (!fill || !parsed || parsed.luminance < 0.52) {
      text.setAttribute("fill", "var(--plain-editor-heading)");
    }
  }
}

function codeLanguageFromClassName(className?: string): string {
  return className
    ?.split(/\s+/)
    .map((item) => item.match(/^language-(.+)$/)?.[1])
    .find(Boolean)
    ?.toLowerCase() ?? "";
}

function codeTextFromChildren(children: ReactNode): string {
  if (typeof children === "string" || typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map(codeTextFromChildren).join("");
  if (isValidElement<{ children?: ReactNode }>(children)) {
    return codeTextFromChildren(children.props.children);
  }
  return "";
}

function markdownUrlTransform(url: string, filePath?: string | null): string {
  if (url.startsWith("#")) return url;
  return safeHref(url, filePath) ?? "";
}

function safeHref(rawHref: string, filePath?: string | null): string | null {
  const href = rawHref.trim();
  if (!href) return null;
  const localPath = resolveLocalMarkdownPath(href, filePath);
  if (localPath) return convertFileSrc(localPath);
  if (href.startsWith("#")) return href;
  try {
    const url = new URL(href);
    if (
      url.protocol === "http:" ||
      url.protocol === "https:" ||
      url.protocol === "mailto:" ||
      url.protocol === "file:" ||
      url.protocol === "asset:"
    ) {
      return href;
    }
  } catch {
    return null;
  }
  return null;
}

function isRenderableMarkdownImageSrc(rawSrc: string, filePath?: string | null): boolean {
  const src = rawSrc.trim();
  if (!src) return false;
  if (resolveLocalMarkdownPath(src, filePath)) return true;
  if (src.startsWith("data:") || src.startsWith("blob:") || src.startsWith("asset:")) return true;
  try {
    const url = new URL(src);
    return url.protocol === "http:" || url.protocol === "https:" || url.protocol === "file:";
  } catch {
    return false;
  }
}

function useResolvedImageSrc(rawSrc: string, filePath?: string | null): string {
  const fallback = useMemo(() => resolveImageSrc(rawSrc, filePath), [filePath, rawSrc]);
  const [src, setSrc] = useState(fallback);

  useEffect(() => {
    let cancelled = false;
    setSrc(fallback);
    const localPath = localImagePath(rawSrc, filePath);
    if (!localPath) return;
    readLocalImageDataUrl(localPath)
      .then((dataUrl) => {
        if (!cancelled) setSrc(dataUrl);
      })
      .catch(() => {
        if (!cancelled) setSrc(fallback);
      });
    return () => {
      cancelled = true;
    };
  }, [fallback, filePath, rawSrc]);

  return src;
}

function resolveImageSrc(rawSrc: string, filePath?: string | null): string {
  const src = rawSrc.trim();
  const localPath = resolveLocalMarkdownPath(src, filePath);
  if (localPath) return convertFileSrc(localPath);
  if (src.startsWith("data:") || src.startsWith("blob:") || src.startsWith("asset:")) return src;
  if (/^file:\/\//i.test(src)) return convertFileSrc(decodeFileUri(src));
  try {
    const url = new URL(src);
    if (url.protocol === "http:" || url.protocol === "https:") return src;
  } catch {
    return src;
  }
  return src;
}

function localImagePath(rawSrc: string, filePath?: string | null): string | null {
  const src = rawSrc.trim();
  const localPath = resolveLocalMarkdownPath(src, filePath);
  if (localPath) return localPath;
  if (/^file:\/\//i.test(src)) return decodeFileUri(src);
  return null;
}

function resolveLocalMarkdownPath(rawPath: string, filePath?: string | null): string | null {
  const value = rawPath.trim().replace(/^<|>$/g, "");
  if (!value || value.startsWith("#")) return null;
  const assetPath = decodeAssetUri(value);
  if (assetPath) return assetPath;
  if (/^(data:|blob:|https?:|mailto:|tel:)/i.test(value)) return null;
  if (/^file:\/\//i.test(value)) return decodeFileUri(value);
  if (/^[A-Za-z]:[\\/]/.test(value) || value.startsWith("/")) return value;
  if (!filePath) return null;
  if (!value.startsWith("./") && !value.startsWith("../") && value.includes(":")) return null;
  return joinMarkdownPath(markdownDirname(filePath), value);
}

function markdownDirname(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const index = normalized.lastIndexOf("/");
  if (index < 0) return ".";
  if (index === 0) return "/";
  return normalized.slice(0, index);
}

function joinMarkdownPath(baseDir: string, relativePath: string): string {
  const normalizedBase = baseDir.replace(/\\/g, "/");
  const normalizedRelative = relativePath.replace(/\\/g, "/");
  const isWindows = /^[A-Za-z]:/.test(normalizedBase);
  const drive = isWindows ? normalizedBase.slice(0, 2) : "";
  const root = isWindows ? "" : normalizedBase.startsWith("/") ? "/" : "";
  const baseParts = (isWindows ? normalizedBase.slice(2) : normalizedBase)
    .split("/")
    .filter(Boolean);
  const relParts = normalizedRelative.split("/");
  const parts = [...baseParts];

  for (const part of relParts) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (parts.length > 0) parts.pop();
      continue;
    }
    parts.push(part);
  }

  const joined = parts.join("/");
  if (isWindows) return `${drive}\\${joined.replace(/\//g, "\\")}`;
  return `${root}${joined}`;
}

function decodeFileUri(uri: string): string {
  try {
    return decodeURIComponent(uri.replace(/^file:\/\//i, ""));
  } catch {
    return uri.replace(/^file:\/\//i, "");
  }
}

function decodeAssetUri(uri: string): string | null {
  const match = uri.match(/^asset:\/\/localhost\/(.+)$/i);
  if (!match) return null;
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return match[1];
  }
}
