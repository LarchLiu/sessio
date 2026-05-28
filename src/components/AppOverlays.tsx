import type { SessionInfo, SessionScope } from "../api";
import { useI18n } from "../i18n";
import { SessionMetaList } from "../pages/ProjectPage";
import ConfirmPopover from "./ConfirmPopover";
import ProjectMemorySearchDialog from "./ProjectMemorySearchDialog";

export type DeleteTarget =
  | { kind: "session"; session: SessionInfo; pos: { x: number; y: number } }
  | { kind: "scope"; scope: SessionScope; pos: { x: number; y: number } };

type AppOverlaysProps = {
  selected: SessionInfo | null;
  metaPopoverMounted: boolean;
  metaPopoverOpen: boolean;
  memorySearchMounted: boolean;
  memorySearchOpen: boolean;
  projectSearchInitialKey: string | undefined;
  memorySearchProjects: Array<{ key: string; label: string }>;
  activeMemorySearchProjectKey: string | null;
  deleteTarget: DeleteTarget | null;
  onCloseMetaPopover: () => void;
  onMetaPopoverExited: () => void;
  onCloseMemorySearch: () => void;
  onMemorySearchExited: () => void;
  onCancelDelete: () => void;
  onConfirmDelete: () => void;
};

export default function AppOverlays({
  selected,
  metaPopoverMounted,
  metaPopoverOpen,
  memorySearchMounted,
  memorySearchOpen,
  projectSearchInitialKey,
  memorySearchProjects,
  activeMemorySearchProjectKey,
  deleteTarget,
  onCloseMetaPopover,
  onMetaPopoverExited,
  onCloseMemorySearch,
  onMemorySearchExited,
  onCancelDelete,
  onConfirmDelete,
}: AppOverlaysProps) {
  const { t } = useI18n();

  return (
    <>
      {selected && metaPopoverMounted && (
        <>
          <button
            type="button"
            data-tauri-drag-region="false"
            aria-label="Close metadata"
            className={
              "absolute inset-x-0 top-12 bottom-0 z-30 bg-bg/35 backdrop-blur-sm transition-opacity duration-150 " +
              (metaPopoverOpen ? "opacity-100" : "opacity-0")
            }
            onClick={onCloseMetaPopover}
            onTransitionEnd={(event) => {
              if (!metaPopoverOpen && event.currentTarget === event.target) {
                onMetaPopoverExited();
              }
            }}
          />
          <div
            data-tauri-drag-region="false"
            className={
              "absolute left-1/2 top-12 z-40 w-[520px] max-w-[calc(100vw-80px)] -translate-x-1/2 transition-[opacity,transform] duration-150 ease-out " +
              (metaPopoverOpen ? "translate-y-0 opacity-100" : "-translate-y-3 opacity-0")
            }
          >
            <SessionMetaList session={selected} />
          </div>
        </>
      )}

      {memorySearchMounted && projectSearchInitialKey && (
        <ProjectMemorySearchDialog
          open={memorySearchOpen}
          initialProjectKey={projectSearchInitialKey}
          projects={memorySearchProjects}
          activeProjectKey={activeMemorySearchProjectKey}
          onClose={onCloseMemorySearch}
          onExited={onMemorySearchExited}
        />
      )}

      {deleteTarget && (
        <ConfirmPopover
          title={t("delete.title")}
          body={
            deleteTarget.kind === "session"
              ? t("delete.session_body")
              : t("delete.scope_body")
          }
          pos={deleteTarget.pos}
          onCancel={onCancelDelete}
          onConfirm={onConfirmDelete}
        />
      )}
    </>
  );
}
