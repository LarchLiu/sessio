import {
  ActionPlacement,
  type ToolbarContext,
  type ToolbarModuleConfig,
  ToolbarModuleExtension,
} from "@blocksuite/affine/shared/services";
import { BlockFlavourIdentifier } from "@blocksuite/std";
import { html } from "lit";

export const CANVAS_SNAPSHOT_SELECTION_EVENT = "sessio:canvas-snapshot-selection";

export type CanvasSnapshotSelectionEventDetail = {
  elementIds: string[];
};

const SnapshotIcon = () => html`
  <svg
    aria-hidden="true"
    fill="none"
    height="20"
    viewBox="0 0 20 20"
    width="20"
    xmlns="http://www.w3.org/2000/svg"
  >
    <path
      d="M4.25 5.75h2.34l1.04-1.5h4.74l1.04 1.5h2.34a1.5 1.5 0 0 1 1.5 1.5v6.5a1.5 1.5 0 0 1-1.5 1.5H4.25a1.5 1.5 0 0 1-1.5-1.5v-6.5a1.5 1.5 0 0 1 1.5-1.5Z"
      stroke="currentColor"
      stroke-linejoin="round"
      stroke-width="1.5"
    />
    <path
      d="M10 12.75a2.25 2.25 0 1 0 0-4.5 2.25 2.25 0 0 0 0 4.5Z"
      stroke="currentColor"
      stroke-width="1.5"
    />
  </svg>
`;

function requestSelectionSnapshot(ctx: ToolbarContext) {
  const event = new CustomEvent<CanvasSnapshotSelectionEventDetail>(CANVAS_SNAPSHOT_SELECTION_EVENT, {
    bubbles: true,
    composed: true,
    detail: {
      elementIds: ctx.getSurfaceModels().map((model) => model.id),
    },
  });
  ctx.host.dispatchEvent(
    event,
  );
  window.dispatchEvent(
    new CustomEvent<CanvasSnapshotSelectionEventDetail>(CANVAS_SNAPSHOT_SELECTION_EVENT, {
      detail: event.detail,
    }),
  );
}

const canvasSurfaceToolbarConfig = {
  actions: [
    {
      placement: ActionPlacement.Start,
      id: "a.snapshot",
      label: "Snapshot",
      showLabel: true,
      tooltip: "Snapshot",
      icon: SnapshotIcon(),
      when: ctx => ctx.getSurfaceModels().length > 0,
      run: requestSelectionSnapshot,
    },
  ],
  when: ctx => ctx.getSurfaceModels().length > 0,
} as const satisfies ToolbarModuleConfig;

export const SessioCanvasToolbarExtension = ToolbarModuleExtension({
  id: BlockFlavourIdentifier("custom:affine:surface:*"),
  config: canvasSurfaceToolbarConfig,
});
