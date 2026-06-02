import { AlertCircle, Bot, Code2, FileText, KeyRound, User, Wrench } from "lucide-react";
import type { ReactNode } from "react";
import type { SessionContentBlock, SessionHistoryRenderBlock, SessionHistoryTurn } from "../api";
import { MarkdownContent, type MarkdownImage } from "../pages/ChatPage";
import ScrollArea from "./ScrollArea";

const noopPreviewImage = (_image: MarkdownImage) => {};

export default function SessionHistoryReadonly({
  turns,
}: {
  turns: SessionHistoryTurn[];
}) {
  if (turns.length === 0) {
    return (
      <div className="rounded-md border border-dashed border-card-border/[0.12] px-3 py-4 text-body-sm text-ink/35">
        No history is available yet.
      </div>
    );
  }
  return (
    <div className="grid gap-2">
      {turns.map((turn) => (
        <div key={turn.turnId} className="grid gap-2">
          {turn.blocks.map((block, index) => (
            <HistoryBlock key={`${turn.turnId}:${index}`} block={block} />
          ))}
        </div>
      ))}
    </div>
  );
}

function HistoryBlock({ block }: { block: SessionHistoryRenderBlock }) {
  switch (block.kind) {
    case "user":
    case "assistant":
    case "thought": {
      const isUser = block.kind === "user";
      const isThought = block.kind === "thought";
      const Icon = isUser ? User : isThought ? Code2 : Bot;
      return (
        <div
          className={
            "rounded-md border px-3 py-2 " +
            (isUser
              ? "border-ink/[0.06] bg-ink/[0.055]"
              : isThought
                ? "border-card-border/[0.10] bg-card-panel text-ink/55"
                : "border-card-border/[0.10] bg-card")
          }
        >
          <div className="mb-1.5 flex items-center gap-2 text-caption uppercase text-ink/35">
            <Icon className="h-3.5 w-3.5" />
            {block.kind}
          </div>
          <div className="grid gap-2 text-body-sm leading-relaxed text-ink/75">
            {block.blocks.map((content, index) => (
              <ContentBlock key={index} block={content} />
            ))}
          </div>
        </div>
      );
    }
    case "tool":
      return <CompactEvent icon={<Wrench className="h-3.5 w-3.5" />} label="Tool" detail={block.toolId} />;
    case "permission":
      return <CompactEvent icon={<KeyRound className="h-3.5 w-3.5" />} label="Permission" detail={block.requestId} />;
    case "sessionUpdate":
      return <CompactEvent icon={<FileText className="h-3.5 w-3.5" />} label={block.updateType} detail="Session update" />;
    case "error":
      return <CompactEvent icon={<AlertCircle className="h-3.5 w-3.5" />} label="Error" detail={block.error.message ?? "Runtime error"} />;
  }
}

function ContentBlock({ block }: { block: SessionContentBlock }) {
  if (block.type === "text") {
    return <MarkdownContent text={stringValue(block.text)} onPreviewImage={noopPreviewImage} />;
  }
  if (block.type === "image") {
    const uri = typeof block.uri === "string" ? block.uri : "";
    const data = typeof block.data === "string" ? block.data : "";
    const mimeType = typeof block.mimeType === "string" ? block.mimeType : "image";
    const src = uri || (data ? `data:${mimeType};base64,${data}` : "");
    return src ? (
      <img src={src} alt={mimeType} className="max-h-64 max-w-full rounded-md border border-card-border/[0.10] object-contain" />
    ) : (
      <PlainObject value={block} />
    );
  }
  if (block.type === "resource" || block.type === "resource_link") {
    const label =
      block.type === "resource_link"
        ? firstString(block.title, block.name, block.uri) ?? "Resource"
        : firstString(block.name, block.uri) ?? "Embedded resource";
    const mimeType = "mimeType" in block ? firstString(block.mimeType) : null;
    const text = block.type === "resource" ? firstString(block.text) : null;
    return (
      <div className="rounded-md border border-card-border/[0.10] bg-card-panel px-2.5 py-2">
        <div className="truncate font-medium text-ink/70">{label}</div>
        {mimeType && <div className="text-caption text-ink/35">{mimeType}</div>}
        {text && (
          <div className="mt-2 text-caption text-ink/55">
            <MarkdownContent text={text} onPreviewImage={noopPreviewImage} />
          </div>
        )}
      </div>
    );
  }
  return <PlainObject value={block} />;
}

function firstString(...values: unknown[]): string | null {
  for (const value of values) {
    if (typeof value === "string" && value.trim()) return value;
  }
  return null;
}

function stringValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value == null) return "";
  return JSON.stringify(value, null, 2);
}

function PlainObject({ value }: { value: unknown }) {
  return (
    <ScrollArea className="max-h-48 min-w-0 rounded-md border border-card-border/[0.10] bg-card-panel" viewportClassName="p-2" orientation="horizontal">
      <pre className="whitespace-pre-wrap break-words font-mono text-caption text-ink/50">
        {JSON.stringify(value, null, 2)}
      </pre>
    </ScrollArea>
  );
}

function CompactEvent({
  icon,
  label,
  detail,
}: {
  icon: ReactNode;
  label: string;
  detail: string;
}) {
  return (
    <div className="flex items-center gap-2 rounded-md border border-card-border/[0.10] bg-card-panel px-3 py-2 text-caption text-ink/45">
      {icon}
      <span className="font-medium text-ink/55">{label}</span>
      <span className="truncate">{detail}</span>
    </div>
  );
}
