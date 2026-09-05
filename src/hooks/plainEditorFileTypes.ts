const EDITABLE_DOCUMENT_EXTENSIONS = new Set([
  "adoc",
  "asciidoc",
  "htm",
  "html",
  "markdown",
  "md",
  "mdown",
  "mdx",
  "mkd",
  "org",
  "qmd",
  "rst",
  "text",
  "txt",
]);

const EDITABLE_DOCUMENT_BASENAMES = new Set([
  "authors",
  "changelog",
  "contributors",
  "copying",
  "license",
  "notice",
  "readme",
  "todo",
]);

const MARKDOWN_DOCUMENT_EXTENSIONS = new Set([
  "markdown",
  "md",
  "mdown",
  "mdx",
  "mkd",
  "qmd",
]);

const HTML_DOCUMENT_EXTENSIONS = new Set(["htm", "html"]);

function plainEditorFileParts(path: string | null | undefined): {
  fileName: string;
  extension: string | null;
} {
  if (!path) return { fileName: "", extension: null };
  const cleanPath = path.split(/[?#]/, 1)[0] ?? "";
  const fileName = cleanPath.split(/[\\/]/).pop()?.trim().toLowerCase() ?? "";
  if (!fileName) return { fileName: "", extension: null };
  const extensionIndex = fileName.lastIndexOf(".");
  if (extensionIndex > 0 && extensionIndex < fileName.length - 1) {
    return { fileName, extension: fileName.slice(extensionIndex + 1) };
  }
  return { fileName, extension: null };
}

export function isPlainEditorEditableDocumentPath(path: string | null | undefined): boolean {
  const { fileName, extension } = plainEditorFileParts(path);
  if (!fileName) return false;
  if (extension) return EDITABLE_DOCUMENT_EXTENSIONS.has(extension);
  return EDITABLE_DOCUMENT_BASENAMES.has(fileName);
}

export function isPlainEditorMarkdownDocumentPath(path: string | null | undefined): boolean {
  const { extension } = plainEditorFileParts(path);
  return extension ? MARKDOWN_DOCUMENT_EXTENSIONS.has(extension) : false;
}

export function isPlainEditorHtmlDocumentPath(path: string | null | undefined): boolean {
  const { extension } = plainEditorFileParts(path);
  return extension ? HTML_DOCUMENT_EXTENSIONS.has(extension) : false;
}

export function isPlainEditorPreviewableDocumentPath(
  path: string | null | undefined,
): boolean {
  return isPlainEditorMarkdownDocumentPath(path) || isPlainEditorHtmlDocumentPath(path);
}
