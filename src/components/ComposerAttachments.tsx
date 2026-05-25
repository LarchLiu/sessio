import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { open } from "@tauri-apps/plugin-dialog";
import { FileText, Image as ImageIcon, X } from "lucide-react";
import {
  type AgentAttachment,
  readLocalImageDataUrl,
  type RuntimeCapabilitySet,
} from "../api";

export type ComposerAttachment = AgentAttachment & {
  name: string;
};

export type AttachmentMenuKey = "images" | "files";

export interface ComposerAttachmentMenuOption {
  key: AttachmentMenuKey;
  label: string;
  icon: React.ReactNode;
}

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

  const addAttachmentPaths = useCallback(
    async (paths: string[], kind: ComposerAttachment["kind"]) => {
      if (paths.length === 0) return;
      const next = paths.map((path) => ({
        kind,
        path,
        mimeType: null,
        name: basename(path),
        previewDataUrl: null,
      }));
      setAttachments((current) => dedupeComposerAttachments([...current, ...next]));
      if (kind !== "image") return;
      const previews = await Promise.allSettled(paths.map((path) => readLocalImageDataUrl(path)));
      setAttachments((current) =>
        current.map((attachment) => {
          if (attachment.kind !== "image") return attachment;
          const index = paths.indexOf(attachment.path);
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

  return {
    attachments,
    supportsAttachments,
    supportsImageAttachments,
    supportsEmbeddedContext,
    removeAttachment,
    clearAttachments,
    pickAttachments,
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
}: {
  attachments: ComposerAttachment[];
  onRemove: (path: string) => void;
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
}: {
  anchor: HTMLButtonElement;
  options: ComposerAttachmentMenuOption[];
  onSelect: (key: AttachmentMenuKey) => void;
  onClose: () => void;
}) {
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const updatePosition = useCallback(() => {
    const rect = anchor.getBoundingClientRect();
    const menuWidth = menuRef.current?.offsetWidth ?? 192;
    const menuHeight = menuRef.current?.offsetHeight ?? 8 + options.length * 40;
    const left = Math.round(
      Math.max(8, Math.min(rect.left + rect.width / 2 - menuWidth / 2, window.innerWidth - menuWidth - 8)),
    );
    const top = Math.round(Math.max(8, rect.top - menuHeight - 10));
    setPos({ top, left });
  }, [anchor, options.length]);

  useLayoutEffect(() => {
    updatePosition();
  }, [updatePosition]);

  useEffect(() => {
    const reposition = () => updatePosition();
    window.addEventListener("scroll", reposition, true);
    window.addEventListener("resize", reposition);
    return () => {
      window.removeEventListener("scroll", reposition, true);
      window.removeEventListener("resize", reposition);
    };
  }, [updatePosition]);

  return createPortal(
    <>
      <div className="fixed inset-0 z-[39] bg-transparent" onMouseDown={onClose} />
      <div
        ref={menuRef}
        className="fixed z-40 min-w-[192px] rounded-xl border border-ink/10 bg-surface-panel p-1.5 shadow-[0_20px_60px_rgba(0,0,0,0.22)]"
        style={{
          top: pos?.top ?? -9999,
          left: pos?.left ?? -9999,
          visibility: pos ? "visible" : "hidden",
        }}
        role="menu"
      >
        {options.map((option) => (
          <button
            key={option.key}
            type="button"
            className="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-body-sm text-ink/72 transition hover:bg-ink/6 hover:text-ink"
            role="menuitem"
            onClick={() => {
              onSelect(option.key);
              onClose();
            }}
          >
            <span className="shrink-0 text-ink/55">{option.icon}</span>
            <span>{option.label}</span>
          </button>
        ))}
      </div>
    </>,
    document.body,
  );
}

function ComposerImageAttachmentCard({
  attachment,
  onRemove,
}: {
  attachment: ComposerAttachment;
  onRemove: (path: string) => void;
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
        <img
          src={src}
          alt={attachment.name}
          className="h-full w-full object-cover"
        />
      ) : (
        <span className="flex h-full w-full items-center justify-center bg-ink/[0.035] text-ink/40">
          <ImageIcon className="h-5 w-5" />
        </span>
      )}
      <button
        type="button"
        onClick={() => onRemove(attachment.path)}
        className="absolute right-1 top-1 rounded-full bg-ink text-[rgb(var(--color-bg-panel))] p-0.5 transition hover:bg-ink/75"
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
