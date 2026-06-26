import {
  useCallback,
  useEffect,
  useState,
} from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FileText, Image as ImageIcon, X } from "lucide-react";
import {
  type AgentAttachment,
  readLocalImageDataUrl,
  type RuntimeCapabilitySet,
  savePastedAttachment,
} from "../api";
import PopupMenu, { type PopupMenuOption, type PopupMenuPlacement } from "./PopupMenu";

export type ComposerAttachment = AgentAttachment & {
  name: string;
};

export type ComposerImageAttachmentPreview = {
  alt: string;
  src: string;
};

export type AttachmentMenuKey = "images" | "files";

export type ComposerAttachmentMenuOption = PopupMenuOption<AttachmentMenuKey>;

const TEXT_ATTACHMENT_EXTENSIONS = [
  "txt",
  "md",
  "markdown",
  "rst",
  "json",
  "jsonl",
  "yaml",
  "yml",
  "toml",
  "xml",
  "csv",
  "ts",
  "tsx",
  "js",
  "jsx",
  "mjs",
  "cjs",
  "py",
  "rs",
  "go",
  "java",
  "kt",
  "swift",
  "rb",
  "php",
  "css",
  "scss",
  "sass",
  "less",
  "html",
  "htm",
  "sh",
  "zsh",
  "bash",
  "sql",
  "c",
  "h",
  "cpp",
  "hpp",
  "cs",
  "lua",
  "pl",
  "r",
  "ex",
  "exs",
  "erl",
  "clj",
  "scala",
  "dart",
  "vue",
  "svelte",
  "dockerfile",
  "gitignore",
  "env",
];

const TEXT_ATTACHMENT_EXTENSION_SET = new Set(TEXT_ATTACHMENT_EXTENSIONS);
const TEXT_ATTACHMENT_MIME_TYPES = new Set([
  "application/json",
  "application/toml",
  "application/xml",
  "application/yaml",
  "application/x-yaml",
  "application/javascript",
  "application/typescript",
  "application/x-sh",
  "application/sql",
]);
const IMAGE_ATTACHMENT_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "heif"];
const IMAGE_ATTACHMENT_EXTENSION_SET = new Set(IMAGE_ATTACHMENT_EXTENSIONS);
const MAX_PASTED_IMAGE_BYTES = 24 * 1024 * 1024;
const MAX_PASTED_FILE_BYTES = 2 * 1024 * 1024;

type ComposerAttachmentDraft = {
  path: string;
  kind: ComposerAttachment["kind"];
  name?: string;
  mimeType?: string | null;
  previewDataUrl?: string | null;
  displayName?: string | null;
};

export type { ComposerAttachmentDraft };

export function useComposerAttachments({
  capabilities,
  onError,
}: {
  capabilities: RuntimeCapabilitySet | null | undefined;
  onError: (message: string) => void;
}) {
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);
  const supportsAttachments = capabilities?.supportsAttachments ?? false;
  const supportsImageAttachments = capabilities?.supportsImageAttachments ?? false;
  const supportsEmbeddedContext = capabilities?.supportsEmbeddedContext ?? false;

  useEffect(() => {
    setAttachments((current) =>
      current.filter((attachment) => {
        if (attachment.kind === "image") return supportsImageAttachments;
        return supportsEmbeddedContext;
      }),
    );
  }, [supportsEmbeddedContext, supportsImageAttachments]);

  const addAttachments = useCallback(
    async (items: ComposerAttachmentDraft[]) => {
      if (items.length === 0) return;
      const next = items.map((item) => ({
        kind: item.kind,
        path: item.path,
        mimeType: item.mimeType ?? null,
        name: item.name ?? basename(item.path),
        previewDataUrl: item.previewDataUrl ?? null,
        displayName: item.displayName ?? item.name ?? basename(item.path),
      }));
      setAttachments((current) => dedupeComposerAttachments([...current, ...next]));
      const imageAttachments = next.filter(
        (attachment) => attachment.kind === "image" && !attachment.previewDataUrl,
      );
      if (imageAttachments.length === 0) return;
      const previews = await Promise.allSettled(
        imageAttachments.map((attachment) => readLocalImageDataUrl(attachment.path)),
      );
      setAttachments((current) =>
        current.map((attachment) => {
          if (attachment.kind !== "image") return attachment;
          const index = imageAttachments.findIndex((item) => item.path === attachment.path);
          if (index < 0) return attachment;
          const preview = previews[index];
          return {
            ...attachment,
            previewDataUrl: preview.status === "fulfilled" ? preview.value : null,
          };
        }),
      );
    },
    [],
  );

  const addAttachmentPaths = useCallback(
    async (paths: string[], kind: ComposerAttachment["kind"]) => {
      await addAttachments(
        paths.map((path) => ({
          kind,
          path,
          mimeType: null,
          name: basename(path),
          previewDataUrl: null,
        })),
      );
    },
    [addAttachments],
  );

  const removeAttachment = useCallback((path: string) => {
    setAttachments((current) => current.filter((attachment) => attachment.path !== path));
  }, []);

  const clearAttachments = useCallback(() => {
    setAttachments([]);
  }, []);

  const pickAttachments = useCallback(
    async (kind: AttachmentMenuKey) => {
      try {
        const selection = await open({
          multiple: true,
          directory: false,
          filters:
            kind === "images"
              ? [
                  {
                    name: "Images",
                    extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "heif"],
                  },
                ]
              : [
                  {
                    name: "Documents and code",
                    extensions: [...TEXT_ATTACHMENT_EXTENSIONS],
                  },
                ],
        });
        if (!selection) return;
        const paths = Array.isArray(selection) ? selection : [selection];
        await addAttachmentPaths(paths, kind === "images" ? "image" : "file");
      } catch (error) {
        onError(`Failed to open file picker: ${String(error)}`);
      }
    },
    [addAttachmentPaths, onError],
  );

  const pasteAttachments = useCallback(
    (clipboardData: DataTransfer | null) => {
      if (!supportsAttachments) return false;
      const files = pastedClipboardFiles(clipboardData);
      if (files.length === 0) return false;
      void (async () => {
        try {
          const drafts: ComposerAttachmentDraft[] = [];
          const rejected: string[] = [];
          for (const file of files) {
            const kind = pastedAttachmentKind(file);
            const name = file.name || defaultPastedFileName(file);
            const mimeType = file.type || null;
            if (kind === "image") {
              if (!supportsImageAttachments) {
                rejected.push(`${name}: image attachments are not supported by this agent.`);
                continue;
              }
              if (file.size > MAX_PASTED_IMAGE_BYTES) {
                rejected.push(`${name}: image is too large.`);
                continue;
              }
            } else if (kind === "file") {
              if (!supportsEmbeddedContext) {
                rejected.push(`${name}: file attachments are not supported by this agent.`);
                continue;
              }
              if (file.size > MAX_PASTED_FILE_BYTES) {
                rejected.push(`${name}: file is too large.`);
                continue;
              }
            } else {
              rejected.push(`${name}: unsupported pasted file type.`);
              continue;
            }

            const { path } = await savePastedAttachment({
              fileName: name,
              mimeType,
              dataBase64: await fileToBase64(file),
            });
            drafts.push({
              path,
              kind,
              name,
              mimeType,
              previewDataUrl: null,
              displayName: name,
            });
          }
          if (drafts.length > 0) await addAttachments(drafts);
          if (rejected.length > 0) {
            const suffix = rejected.length > 1 ? ` (+${rejected.length - 1} more)` : "";
            onError(`Some pasted attachments were skipped. ${rejected[0]}${suffix}`);
          }
        } catch (error) {
          onError(`Failed to paste attachment: ${String(error)}`);
        }
      })();
      return true;
    },
    [
      addAttachments,
      onError,
      supportsAttachments,
      supportsEmbeddedContext,
      supportsImageAttachments,
    ],
  );

  return {
    attachments,
    supportsAttachments,
    supportsImageAttachments,
    supportsEmbeddedContext,
    addAttachments,
    removeAttachment,
    clearAttachments,
    pickAttachments,
    pasteAttachments,
  };
}

export function attachmentMenuOptions({
  supportsImageAttachments,
  supportsEmbeddedContext,
  imageLabel,
  fileLabel,
}: {
  supportsImageAttachments: boolean;
  supportsEmbeddedContext: boolean;
  imageLabel: string;
  fileLabel: string;
}): ComposerAttachmentMenuOption[] {
  const options: ComposerAttachmentMenuOption[] = [];
  if (supportsImageAttachments) {
    options.push({
      key: "images",
      label: imageLabel,
      icon: <ImageIcon className="h-4 w-4" />,
    });
  }
  if (supportsEmbeddedContext) {
    options.push({
      key: "files",
      label: fileLabel,
      icon: <FileText className="h-4 w-4" />,
    });
  }
  return options;
}

export function ComposerAttachmentPreviewList({
  attachments,
  onRemove,
  onPreviewImage,
}: {
  attachments: ComposerAttachment[];
  onRemove: (path: string) => void;
  onPreviewImage?: (image: ComposerImageAttachmentPreview) => void;
}) {
  if (attachments.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-2 border-b border-ink/5 px-3.5 pt-3 pb-2">
      {attachments.map((attachment) =>
        attachment.kind === "image" ? (
          <ComposerImageAttachmentCard
            key={attachment.path}
            attachment={attachment}
            onRemove={onRemove}
            onPreviewImage={onPreviewImage}
          />
        ) : (
          <ComposerFileAttachmentCard
            key={attachment.path}
            attachment={attachment}
            onRemove={onRemove}
          />
        ),
      )}
    </div>
  );
}

export function ComposerAttachmentMenu({
  anchor,
  options,
  onSelect,
  onClose,
  placement = "top",
}: {
  anchor: HTMLButtonElement;
  options: ComposerAttachmentMenuOption[];
  onSelect: (key: AttachmentMenuKey) => void;
  onClose: () => void;
  placement?: PopupMenuPlacement;
}) {
  return (
    <PopupMenu
      anchor={anchor}
      options={options}
      placement={placement}
      onSelect={onSelect}
      onClose={onClose}
    />
  );
}

function ComposerImageAttachmentCard({
  attachment,
  onRemove,
  onPreviewImage,
}: {
  attachment: ComposerAttachment;
  onRemove: (path: string) => void;
  onPreviewImage?: (image: ComposerImageAttachmentPreview) => void;
}) {
  const [src, setSrc] = useState<string | null>(attachment.previewDataUrl ?? null);

  useEffect(() => {
    let cancelled = false;
    if (attachment.previewDataUrl) {
      setSrc(attachment.previewDataUrl);
      return;
    }
    setSrc(null);
    readLocalImageDataUrl(attachment.path)
      .then((dataUrl) => {
        if (!cancelled) setSrc(dataUrl);
      })
      .catch(() => {
        if (!cancelled) setSrc(null);
      });
    return () => {
      cancelled = true;
    };
  }, [attachment.path, attachment.previewDataUrl]);

  return (
    <span className="relative block h-16 w-20 overflow-hidden rounded-lg border border-ink/8 bg-bg-panel shadow-sm">
      {src ? (
        <button
          type="button"
          onClick={() =>
            onPreviewImage?.({
              src,
              alt: attachment.displayName ?? attachment.name,
            })
          }
          className="block h-full w-full cursor-zoom-in focus:outline-none focus:ring-2 focus:ring-inset focus:ring-ink/25"
          aria-label={`Preview ${attachment.name}`}
        >
          <img
            src={src}
            alt={attachment.name}
            className="h-full w-full object-cover"
          />
        </button>
      ) : (
        <span className="flex h-full w-full items-center justify-center bg-ink/[0.035] text-ink/40">
          <ImageIcon className="h-5 w-5" />
        </span>
      )}
      <button
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          onRemove(attachment.path);
        }}
        className="absolute right-1 top-1 z-10 rounded-full bg-ink text-[rgb(var(--color-bg-panel))] p-0.5 transition hover:bg-ink/75"
        aria-label={`Remove ${attachment.name}`}
      >
        <X className="h-3 w-3" />
      </button>
    </span>
  );
}

function ComposerFileAttachmentCard({
  attachment,
  onRemove,
}: {
  attachment: ComposerAttachment;
  onRemove: (path: string) => void;
}) {
  return (
    <span className="relative inline-flex min-w-[142px] max-w-[220px] items-center gap-2 rounded-lg border border-ink/8 bg-bg-panel px-3 py-2 pr-8 text-body-sm text-ink/78 shadow-sm">
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-emerald/10 text-emerald">
        <FileText className="h-4 w-4" />
      </span>
      <span className="min-w-0">
        <span className="block truncate font-medium leading-4">{attachment.name}</span>
        <span className="block text-caption uppercase leading-4 text-ink/45">
          {fileExtensionLabel(attachment.name)}
        </span>
      </span>
      <button
        type="button"
        onClick={() => onRemove(attachment.path)}
        className="absolute right-1.5 top-1.5 rounded-full bg-ink text-[rgb(var(--color-bg-panel))] p-0.5 transition hover:bg-ink/75"
        aria-label={`Remove ${attachment.name}`}
      >
        <X className="h-3 w-3" />
      </button>
    </span>
  );
}

function basename(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

function pastedClipboardFiles(clipboardData: DataTransfer | null): File[] {
  if (!clipboardData) return [];
  const files: File[] = [];
  for (const item of Array.from(clipboardData.items ?? [])) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (file) files.push(file);
  }
  for (const file of Array.from(clipboardData.files ?? [])) {
    files.push(file);
  }
  const seen = new Set<string>();
  return files.filter((file) => {
    const key = `${file.name}\0${file.type}\0${file.size}\0${file.lastModified}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function pastedAttachmentKind(file: File): ComposerAttachment["kind"] | null {
  if (isPastedImageFile(file)) return "image";
  if (isSupportedPastedTextFile(file)) return "file";
  return null;
}

function isPastedImageFile(file: File): boolean {
  const mimeType = file.type.toLowerCase();
  if (mimeType.startsWith("image/")) return true;
  const extension = extensionLower(file.name);
  return Boolean(extension && IMAGE_ATTACHMENT_EXTENSION_SET.has(extension));
}

function isSupportedPastedTextFile(file: File): boolean {
  const mimeType = file.type.toLowerCase();
  if (mimeType.startsWith("text/") || TEXT_ATTACHMENT_MIME_TYPES.has(mimeType)) return true;
  const extension = extensionLower(file.name);
  if (extension && TEXT_ATTACHMENT_EXTENSION_SET.has(extension)) return true;
  const name = file.name.toLowerCase();
  return name === "dockerfile" || name === ".gitignore" || name === ".env";
}

function defaultPastedFileName(file: File): string {
  const extension = extensionForMime(file.type);
  if (isPastedImageFile(file)) return `pasted-image.${extension ?? "png"}`;
  return extension ? `pasted-file.${extension}` : "pasted-file";
}

async function fileToBase64(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
  }
  return window.btoa(binary);
}

function extensionForMime(mimeType: string | null | undefined): string | null {
  switch (mimeType?.toLowerCase()) {
    case "image/png":
      return "png";
    case "image/jpeg":
      return "jpg";
    case "image/webp":
      return "webp";
    case "image/gif":
      return "gif";
    case "image/svg+xml":
      return "svg";
    case "image/bmp":
      return "bmp";
    case "image/heic":
      return "heic";
    case "image/heif":
      return "heif";
    case "text/plain":
      return "txt";
    case "text/markdown":
      return "md";
    case "text/csv":
      return "csv";
    case "text/html":
      return "html";
    case "text/css":
      return "css";
    case "application/json":
      return "json";
    case "application/xml":
      return "xml";
    case "application/yaml":
    case "application/x-yaml":
      return "yaml";
    case "application/toml":
      return "toml";
    default:
      return null;
  }
}

function extensionLower(name: string): string | null {
  const extension = name.split(".").pop();
  if (!extension || extension === name) return null;
  return extension.toLowerCase();
}

function dedupeComposerAttachments(
  attachments: ComposerAttachment[],
): ComposerAttachment[] {
  const seen = new Set<string>();
  const deduped: ComposerAttachment[] = [];
  for (const attachment of attachments) {
    if (seen.has(attachment.path)) continue;
    seen.add(attachment.path);
    deduped.push(attachment);
  }
  return deduped;
}

function fileExtensionLabel(name: string): string {
  const ext = name.split(".").pop();
  return ext && ext !== name ? ext.toUpperCase() : "Text";
}
