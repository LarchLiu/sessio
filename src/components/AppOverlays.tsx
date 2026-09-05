import { useState } from "react";
import { createPortal } from "react-dom";
import type { SessionInfo, SessionScope, SessioAppInfo } from "../api";
import { useI18n } from "../i18n";
import { SessionMetaList } from "../pages/ProjectPage";
import ConfirmPopover from "./ConfirmPopover";
import ProjectMemorySearchDialog from "./ProjectMemorySearchDialog";
import UpdateConfirmDialog from "./UpdateConfirmDialog";

export type DeleteTarget =
  | { kind: "session"; session: SessionInfo; pos: { x: number; y: number } }
  | {
      kind: "project";
      projectId: string;
      scope: Extract<SessionScope, { kind: "project" }>;
      pos: { x: number; y: number };
    }
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
  appRenameTarget: SessioAppInfo | null;
  appRenameCurrentName: string | null;
  updateConfirmMounted: boolean;
  updateConfirmOpen: boolean;
  updateCurrentVersion: string;
  updateLatestVersion: string | null;
  updateReleaseNotes: string | null;
  updateCanInstall: boolean;
  updateInstalling: boolean;
  updateReady: boolean;
  updateDownloadedBytes: number;
  updateTotalBytes: number | null;
  onCloseMetaPopover: () => void;
  onMetaPopoverExited: () => void;
  onCloseMemorySearch: () => void;
  onMemorySearchExited: () => void;
  onCancelDelete: () => void;
  onConfirmDelete: () => void;
  onCancelAppRename: () => void;
  onConfirmAppRename: (name: string) => void;
  onCancelUpdateConfirm: () => void;
  onConfirmUpdate: () => void;
  onUpdateConfirmExited: () => void;
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
  appRenameTarget,
  appRenameCurrentName,
  updateConfirmMounted,
  updateConfirmOpen,
  updateCurrentVersion,
  updateLatestVersion,
  updateReleaseNotes,
  updateCanInstall,
  updateInstalling,
  updateReady,
  updateDownloadedBytes,
  updateTotalBytes,
  onCloseMetaPopover,
  onMetaPopoverExited,
  onCloseMemorySearch,
  onMemorySearchExited,
  onCancelDelete,
  onConfirmDelete,
  onCancelAppRename,
  onConfirmAppRename,
  onCancelUpdateConfirm,
  onConfirmUpdate,
  onUpdateConfirmExited,
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
              : deleteTarget.kind === "project"
                ? t("delete.project_body")
                : t("delete.scope_body")
          }
          pos={deleteTarget.pos}
          onCancel={onCancelDelete}
          onConfirm={onConfirmDelete}
        />
      )}

      {appRenameTarget && (
        <AppRenameDialog
          key={`${appRenameTarget.id}:${appRenameCurrentName ?? appRenameTarget.slug}`}
          initialName={appRenameCurrentName ?? appRenameTarget.slug}
          onCancel={onCancelAppRename}
          onConfirm={onConfirmAppRename}
        />
      )}

      {updateConfirmMounted && updateLatestVersion && (
        <UpdateConfirmDialog
          open={updateConfirmOpen}
          currentVersion={updateCurrentVersion}
          latestVersion={updateLatestVersion}
          releaseNotes={updateReleaseNotes}
          canInstall={updateCanInstall}
          installing={updateInstalling}
          updateReady={updateReady}
          downloadedBytes={updateDownloadedBytes}
          totalBytes={updateTotalBytes}
          onCancel={onCancelUpdateConfirm}
          onConfirm={onConfirmUpdate}
          onExited={onUpdateConfirmExited}
        />
      )}
    </>
  );
}

function AppRenameDialog({
  initialName,
  onCancel,
  onConfirm,
}: {
  initialName: string;
  onCancel: () => void;
  onConfirm: (name: string) => void;
}) {
  const { t } = useI18n();
  const [name, setName] = useState(initialName);

  return createPortal(
    <div
      className="fixed inset-0 z-[90] flex items-center justify-center bg-black/35 px-4"
      onMouseDown={onCancel}
    >
      <form
        className="w-full max-w-[360px] rounded-xl border border-ink/10 bg-surface-panel p-4 shadow-2xl"
        onSubmit={(event) => {
          event.preventDefault();
          onConfirm(name);
        }}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="text-body font-medium text-ink">{t("apps.rename")}</div>
        <label className="mt-3 block text-caption text-ink/55" htmlFor="app-display-name">
          {t("apps.rename_title")}
        </label>
        <input
          id="app-display-name"
          autoFocus
          value={name}
          onChange={(event) => setName(event.target.value)}
          className="mt-1.5 h-9 w-full rounded-md border border-ink/15 bg-surface px-2.5 text-body-sm text-ink outline-none focus:border-ink/35 focus:ring-2 focus:ring-ink/10"
        />
        <div className="mt-4 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-md px-2.5 py-1.5 text-body-sm text-ink/65 transition hover:bg-ink/5 hover:text-ink"
          >
            {t("apps.rename_cancel")}
          </button>
          <button
            type="submit"
            className="rounded-md bg-ink px-2.5 py-1.5 text-body-sm font-medium text-surface-panel transition hover:bg-ink/85"
          >
            {t("apps.rename_submit")}
          </button>
        </div>
      </form>
    </div>,
    document.body,
  );
}
