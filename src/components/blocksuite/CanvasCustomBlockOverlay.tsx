import type { CSSProperties } from "react";
import { FileCardHost } from "../../lib/blocksuite/blocks/file-card/host";
import { MarkdownPreviewHost } from "../../lib/blocksuite/blocks/markdown-preview/host";
import { WorkflowCardHost } from "../../lib/blocksuite/blocks/workflow-card/host";

type OverlayBase = {
  blockId: string;
  left: number;
  top: number;
  baseWidth: number;
  baseHeight: number;
  scale: number;
  selected: boolean;
};

export type FileCardOverlayItem = OverlayBase & {
  kind: "file_card";
  title: string;
  sourcePath: string;
  sourceType: string;
  subtitle: string;
  summary: string;
  status: string;
};

export type MarkdownPreviewOverlayItem = OverlayBase & {
  kind: "markdown_preview";
  title: string;
  sourcePath: string;
  excerpt: string;
  contentVersion: string;
  renderMode: "summary" | "preview";
  workspacePath: string | null;
};

export type WorkflowCardOverlayItem = OverlayBase & {
  kind: "workflow_card";
  title: string;
  threadId: string;
  threadStageId: string;
  executionState: string;
  lastRunId: string;
  threadGoal: string;
  workflowSummaryMarkdown: string;
};

export type CanvasCustomBlockOverlayItem =
  | FileCardOverlayItem
  | MarkdownPreviewOverlayItem
  | WorkflowCardOverlayItem;

export interface CanvasCustomBlockOverlayProps {
  items: readonly CanvasCustomBlockOverlayItem[];
  onPromoteFileCardToMarkdown: (blockId: string) => void;
  onRunWorkflow: (blockId: string) => void;
  onOpenWorkflowThread: (blockId: string) => void;
  onOpenFile?: (path: string) => void;
  onDragMarkdownPreviewFromHeader?: (
    blockId: string,
    event: React.PointerEvent<HTMLDivElement>,
  ) => void;
  onUpdateMarkdownRenderMode: (
    blockId: string,
    nextMode: "summary" | "preview",
  ) => void;
}

export function CanvasCustomBlockOverlay({
  items,
  onPromoteFileCardToMarkdown,
  onRunWorkflow,
  onOpenWorkflowThread,
  onOpenFile,
  onDragMarkdownPreviewFromHeader,
  onUpdateMarkdownRenderMode,
}: CanvasCustomBlockOverlayProps) {
  if (items.length === 0) {
    return null;
  }

  return (
    <div className="pointer-events-none absolute inset-0 overflow-hidden">
      {items.map((item) => {
        const shellClassName = "absolute";
        const scaledWidth = item.baseWidth * item.scale;
        const scaledHeight = item.baseHeight * item.scale;
        const shellStyle: CSSProperties = {
          left: item.left,
          top: item.top,
          width: scaledWidth,
          height: scaledHeight,
        };
        const contentStyle: CSSProperties & { zoom: number } = {
          width: item.baseWidth,
          height: item.baseHeight,
          zoom: item.scale,
          transformOrigin: "top left",
        };

        if (item.kind === "file_card") {
          return (
            <div key={item.blockId} className={shellClassName + " pointer-events-none"} style={shellStyle}>
              <div style={contentStyle}>
                <FileCardHost
                  title={item.title}
                  sourcePath={item.sourcePath}
                  sourceType={item.sourceType}
                  subtitle={item.subtitle}
                  summary={item.summary}
                  status={item.status}
                  onPromoteToMarkdown={() => onPromoteFileCardToMarkdown(item.blockId)}
                  onOpenFile={onOpenFile}
                  interactionMode="overlay"
                />
              </div>
            </div>
          );
        }

        if (item.kind === "workflow_card") {
          return (
            <div key={item.blockId} className={shellClassName + " pointer-events-none"} style={shellStyle}>
              <div style={contentStyle}>
                <WorkflowCardHost
                  title={item.title}
                  threadId={item.threadId}
                  threadStageId={item.threadStageId}
                  executionState={item.executionState}
                  lastRunId={item.lastRunId}
                  threadGoal={item.threadGoal}
                  workflowSummaryMarkdown={item.workflowSummaryMarkdown}
                  onRunWorkflow={() => onRunWorkflow(item.blockId)}
                  onOpenThread={() => onOpenWorkflowThread(item.blockId)}
                  interactionMode="overlay"
                />
              </div>
            </div>
          );
        }

        return (
          <div key={item.blockId} className={shellClassName + " pointer-events-none"} style={shellStyle}>
            <div style={contentStyle}>
              <MarkdownPreviewHost
                workspacePath={item.workspacePath}
                blockId={item.blockId}
                selected={item.selected}
                title={item.title}
                sourcePath={item.sourcePath}
                excerpt={item.excerpt}
                contentVersion={item.contentVersion}
                renderMode={item.renderMode}
                onOpenFile={onOpenFile}
                onHeaderPointerDown={(event) =>
                  onDragMarkdownPreviewFromHeader?.(item.blockId, event)
                }
                onToggleRenderMode={(nextMode) =>
                  onUpdateMarkdownRenderMode(item.blockId, nextMode)
                }
                interactionMode="overlay"
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}
