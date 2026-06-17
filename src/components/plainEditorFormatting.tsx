import type { Block, BlockNoteEditor } from "@blocknote/core";
import { SideMenuExtension, filterSuggestionItems } from "@blocknote/core/extensions";
import {
  AddBlockButton,
  BasicTextStyleButton,
  CreateLinkButton,
  DragHandleMenu,
  FormattingToolbar,
  FormattingToolbarController,
  getDefaultReactSlashMenuItems,
  getFormattingToolbarItems,
  RemoveBlockItem,
  SideMenu,
  SideMenuController,
  SuggestionMenuController,
  TableColumnHeaderItem,
  TableRowHeaderItem,
  type DefaultReactSuggestionItem,
  type FormattingToolbarProps,
  type SideMenuProps,
  useBlockNoteEditor,
  useComponentsContext,
  useDictionary,
  useExtension,
  useExtensionState,
} from "@blocknote/react";
import {
  CheckSquare,
  Code,
  GripVertical,
  Heading1,
  Heading2,
  Heading3,
  List,
  ListOrdered,
  Minus,
  Pilcrow,
  Quote,
  Table2,
  type LucideIcon,
} from "lucide-react";
import {
  useCallback,
  useLayoutEffect,
  useRef,
  type ComponentType,
  type PointerEvent as ReactPointerEvent,
  type ReactElement,
  type ReactNode,
  type MouseEvent as ReactMouseEvent,
} from "react";

type PlainEditorSuggestionItem = DefaultReactSuggestionItem & { key?: string };
type PlainEditorBlock = Block<any, any, any>;
type DropPlacement = "before" | "after";
type DropTarget = {
  blockId: string;
  element: HTMLElement;
  placement: DropPlacement;
};
type ReorderAffordances = {
  draggedElement: HTMLElement;
  dropIndicator: HTMLElement;
  pointerOffsetX: number;
  pointerOffsetY: number;
  preview: HTMLElement;
  previousDraggedOpacity: string;
};
type PointerReorderState = {
  affordances?: ReorderAffordances;
  clearListeners: () => void;
  draggedBlockId: string;
  editorElement: HTMLElement;
  hasMoved: boolean;
  lastDropTarget?: DropTarget | null;
  ownerDocument: Document;
  pointerId: number;
  startX: number;
  startY: number;
};
type SideMenuAlignmentState = {
  attemptsRemaining: number;
  frame: number | null;
  hasObservedTargets: boolean;
};
type SideMenuAlignmentContext = {
  blockId: string;
  editorElement: HTMLElement;
  observeTargets: () => void;
  ownerWindow: Window;
  retry: () => void;
  state: SideMenuAlignmentState;
};

const UNSUPPORTED_FORMATTING_TOOLBAR_KEYS = new Set([
  "underlineStyleButton",
  "textAlignLeftButton",
  "textAlignCenterButton",
  "textAlignRightButton",
  "colorStyleButton",
]);

const SUPPORTED_SLASH_MENU_KEYS = new Set([
  "paragraph",
  "heading",
  "heading_2",
  "heading_3",
  "quote",
  "bullet_list",
  "numbered_list",
  "check_list",
  "code_block",
  "table",
  "divider",
]);

const SLASH_MENU_ICONS: Partial<Record<string, LucideIcon>> = {
  bullet_list: List,
  check_list: CheckSquare,
  code_block: Code,
  divider: Minus,
  heading: Heading1,
  heading_2: Heading2,
  heading_3: Heading3,
  numbered_list: ListOrdered,
  paragraph: Pilcrow,
  quote: Quote,
  table: Table2,
};
const BLOCK_CONTAINER_SELECTOR = '[data-node-type="blockContainer"][data-id]';
const POINTER_REORDER_THRESHOLD_PX = 4;
const SIDE_MENU_ALIGNMENT_ATTEMPTS = 8;

function createSlashMenuIcon(Icon: LucideIcon) {
  return (
    <span className="sessio-plain-editor-slash-icon">
      <Icon aria-hidden="true" size={18} strokeWidth={1.85} />
    </span>
  );
}

function filterFormattingToolbarItems<T extends ReactElement>(items: T[]): T[] {
  return items.filter(
    (item) => !UNSUPPORTED_FORMATTING_TOOLBAR_KEYS.has(String(item.key)),
  );
}

function getPlainEditorSlashItems(
  editor: BlockNoteEditor,
  query: string,
): PlainEditorSuggestionItem[] {
  const items = getDefaultReactSlashMenuItems(editor) as PlainEditorSuggestionItem[];
  const supportedItems = items
    .filter((item) => SUPPORTED_SLASH_MENU_KEYS.has(String(item.key)))
    .map((item) => {
      const Icon = SLASH_MENU_ICONS[String(item.key)];
      return Icon ? { ...item, icon: createSlashMenuIcon(Icon) } : item;
    });
  return filterSuggestionItems(supportedItems, query);
}

export function PlainEditorFormattingToolbar(props: FormattingToolbarProps) {
  const items = filterFormattingToolbarItems(getFormattingToolbarItems());
  const strikeIndex = items.findIndex((item) => item.key === "strikeStyleButton");
  const nextItems = [...items];
  nextItems.splice(
    strikeIndex === -1 ? nextItems.length : strikeIndex + 1,
    0,
    <BasicTextStyleButton basicTextStyle="code" key="codeStyleButton" />,
  );
  if (!nextItems.some((item) => item.key === "createLinkButton")) {
    nextItems.push(<CreateLinkButton key="createLinkButton" />);
  }

  return (
    <FormattingToolbar {...props}>
      {nextItems}
    </FormattingToolbar>
  );
}

export function PlainEditorFormattingToolbarController() {
  return (
    <FormattingToolbarController
      formattingToolbar={PlainEditorFormattingToolbar}
      floatingUIOptions={{
        useFloatingOptions: {
          placement: "top-start",
        },
      }}
    />
  );
}

export function PlainEditorSlashMenuController() {
  const editor = useBlockNoteEditor();
  const getItems = useCallback(
    async (query: string) => getPlainEditorSlashItems(editor, query),
    [editor],
  );

  return (
    <SuggestionMenuController
      triggerCharacter="/"
      getItems={getItems}
    />
  );
}

function PlainEditorDragHandleMenu() {
  const dict = useDictionary();

  return (
    <DragHandleMenu>
      <RemoveBlockItem>{dict.drag_handle.delete_menuitem}</RemoveBlockItem>
      <TableRowHeaderItem>{dict.drag_handle.header_row_menuitem}</TableRowHeaderItem>
      <TableColumnHeaderItem>{dict.drag_handle.header_column_menuitem}</TableColumnHeaderItem>
    </DragHandleMenu>
  );
}

function editorBlockElement(editor: BlockNoteEditor): HTMLElement | null {
  const element = editor.domElement;
  if (!(element instanceof HTMLElement)) return null;
  return element.matches(".bn-editor")
    ? element
    : element.querySelector(".bn-editor");
}

function blockElementById(editorElement: HTMLElement, blockId: string): HTMLElement | null {
  for (const element of editorElement.querySelectorAll(BLOCK_CONTAINER_SELECTOR)) {
    if (element instanceof HTMLElement && element.dataset.id === blockId) return element;
  }
  return null;
}

function blockElements(editorElement: HTMLElement): HTMLElement[] {
  return Array.from(editorElement.querySelectorAll(BLOCK_CONTAINER_SELECTOR))
    .filter((element): element is HTMLElement => element instanceof HTMLElement);
}

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

function blockElementFromPoint({
  editorElement,
  ownerDocument,
  x,
  y,
}: {
  editorElement: HTMLElement;
  ownerDocument: Document;
  x: number;
  y: number;
}) {
  if (typeof ownerDocument.elementsFromPoint !== "function") return null;
  const editorRect = editorElement.getBoundingClientRect();
  if (editorRect.width <= 0 || editorRect.height <= 0) return null;

  const hitX = clamp(x, editorRect.left + 10, editorRect.right - 10);
  const hitY = clamp(y, editorRect.top + 1, editorRect.bottom - 1);
  for (const element of ownerDocument.elementsFromPoint(hitX, hitY)) {
    if (!editorElement.contains(element)) continue;
    const blockElement = element.closest(BLOCK_CONTAINER_SELECTOR);
    if (blockElement instanceof HTMLElement && editorElement.contains(blockElement)) {
      return blockElement;
    }
  }

  for (const blockElement of blockElements(editorElement)) {
    const rect = blockElement.getBoundingClientRect();
    if (hitY >= rect.top && hitY <= rect.bottom) return blockElement;
  }

  return null;
}

function blockIdFromElement(blockElement: HTMLElement) {
  return blockElement.dataset.id ?? null;
}

function dropPlacementForPoint(blockElement: HTMLElement, y: number): DropPlacement {
  const rect = blockElement.getBoundingClientRect();
  return y < rect.top + rect.height / 2 ? "before" : "after";
}

function hasChildBlock(block: PlainEditorBlock, blockId: string): boolean {
  for (const child of block.children) {
    if (child.id === blockId || hasChildBlock(child, blockId)) return true;
  }
  return false;
}

function liveBlock(editor: BlockNoteEditor, blockId: string): PlainEditorBlock | null {
  try {
    return editor.getBlock(blockId) as PlainEditorBlock | undefined ?? null;
  } catch {
    return null;
  }
}

function styleDragPreview(preview: HTMLElement, rect: DOMRect) {
  preview.setAttribute("aria-hidden", "true");
  preview.className = "sessio-plain-editor-drag-preview";
  preview.style.position = "fixed";
  preview.style.width = `${rect.width}px`;
  preview.style.maxHeight = `${Math.max(rect.height, 1)}px`;
  preview.style.overflow = "hidden";
  preview.style.pointerEvents = "none";
  preview.style.opacity = "0.72";
  preview.style.zIndex = "14000";
  preview.style.boxSizing = "border-box";
  preview.style.borderRadius = "6px";
  preview.style.background = "var(--plain-editor-menu-bg, white)";
  preview.style.boxShadow = "0 10px 26px rgb(0 0 0 / 0.18)";
}

function createDragPreview(draggedElement: HTMLElement, ownerDocument: Document) {
  const preview = ownerDocument.createElement("div");
  const clone = draggedElement.cloneNode(true);
  const rect = draggedElement.getBoundingClientRect();
  if (clone instanceof HTMLElement) {
    clone.style.margin = "0";
    clone.style.width = "100%";
    clone.style.pointerEvents = "none";
    preview.appendChild(clone);
  }
  styleDragPreview(preview, rect);
  ownerDocument.body.appendChild(preview);
  return preview;
}

function createDropIndicator(ownerDocument: Document) {
  const indicator = ownerDocument.createElement("div");
  indicator.className = "sessio-plain-editor-drop-indicator";
  indicator.style.position = "fixed";
  indicator.style.height = "2px";
  indicator.style.pointerEvents = "none";
  indicator.style.background = "var(--plain-editor-accent, #155dff)";
  indicator.style.borderRadius = "999px";
  indicator.style.boxShadow = "0 0 0 1px rgb(21 93 255 / 0.12), 0 0 10px rgb(21 93 255 / 0.28)";
  indicator.style.zIndex = "14001";
  indicator.style.display = "none";
  ownerDocument.body.appendChild(indicator);
  return indicator;
}

function createReorderAffordances(state: PointerReorderState): ReorderAffordances | undefined {
  const draggedElement = blockElementById(state.editorElement, state.draggedBlockId);
  if (!draggedElement) return undefined;
  const rect = draggedElement.getBoundingClientRect();
  const previousDraggedOpacity = draggedElement.style.opacity;
  const preview = createDragPreview(draggedElement, state.ownerDocument);
  draggedElement.style.opacity = "0.35";
  return {
    draggedElement,
    dropIndicator: createDropIndicator(state.ownerDocument),
    pointerOffsetX: state.startX - rect.left,
    pointerOffsetY: state.startY - rect.top,
    preview,
    previousDraggedOpacity,
  };
}

function cleanupReorderAffordances(affordances: ReorderAffordances | undefined) {
  if (!affordances) return;
  affordances.draggedElement.style.opacity = affordances.previousDraggedOpacity;
  affordances.preview.remove();
  affordances.dropIndicator.remove();
}

function updateDragPreview(affordances: ReorderAffordances, x: number, y: number) {
  affordances.preview.style.left = `${x - affordances.pointerOffsetX}px`;
  affordances.preview.style.top = `${y - affordances.pointerOffsetY}px`;
}

function updateDropIndicator(affordances: ReorderAffordances | undefined, target: DropTarget | null) {
  if (!affordances || !target) {
    if (affordances) affordances.dropIndicator.style.display = "none";
    return;
  }
  const rect = target.element.getBoundingClientRect();
  affordances.dropIndicator.style.display = "block";
  affordances.dropIndicator.style.left = `${rect.left}px`;
  affordances.dropIndicator.style.top = `${target.placement === "before" ? rect.top - 1 : rect.bottom - 1}px`;
  affordances.dropIndicator.style.width = `${rect.width}px`;
}

function validDropTarget({
  editor,
  state,
  x,
  y,
}: {
  editor: BlockNoteEditor;
  state: PointerReorderState;
  x: number;
  y: number;
}): DropTarget | null {
  const targetElement = blockElementFromPoint({
    editorElement: state.editorElement,
    ownerDocument: state.ownerDocument,
    x,
    y,
  });
  if (!targetElement) return null;

  const blockId = blockIdFromElement(targetElement);
  if (!blockId || blockId === state.draggedBlockId) return null;

  const draggedBlock = liveBlock(editor, state.draggedBlockId);
  const targetBlock = liveBlock(editor, blockId);
  if (!draggedBlock || !targetBlock || hasChildBlock(draggedBlock, blockId)) return null;

  return {
    blockId,
    element: targetElement,
    placement: dropPlacementForPoint(targetElement, y),
  };
}

function moveBlockByPointerDrop({
  editor,
  draggedBlockId,
  targetBlockId,
  placement,
}: {
  editor: BlockNoteEditor;
  draggedBlockId: string;
  targetBlockId: string;
  placement: DropPlacement;
}) {
  if (draggedBlockId === targetBlockId) return false;
  const draggedBlock = liveBlock(editor, draggedBlockId);
  const targetBlock = liveBlock(editor, targetBlockId);
  if (!draggedBlock || !targetBlock || hasChildBlock(draggedBlock, targetBlockId)) return false;

  let moved = false;
  editor.focus();
  editor.transact(() => {
    const currentDraggedBlock = liveBlock(editor, draggedBlockId);
    const currentTargetBlock = liveBlock(editor, targetBlockId);
    if (!currentDraggedBlock || !currentTargetBlock) return;
    if (hasChildBlock(currentDraggedBlock, targetBlockId)) return;
    editor.removeBlocks([currentDraggedBlock.id]);
    editor.insertBlocks(
      [currentDraggedBlock] as Parameters<typeof editor.insertBlocks>[0],
      currentTargetBlock.id,
      placement,
    );
    moved = true;
  });
  return moved;
}

function sideMenuElementForEditor(editorElement: HTMLElement): HTMLElement | null {
  const container = editorElement.closest(".bn-container") ?? editorElement.parentElement ?? editorElement;
  const sideMenu = container.querySelector(".bn-side-menu");
  return sideMenu instanceof HTMLElement ? sideMenu : null;
}

function blockTextAnchorRect(blockElement: HTMLElement): DOMRect | null {
  const content = blockElement.querySelector(".bn-block-content");
  const inlineContent = content?.querySelector(".bn-inline-content") ?? content;
  if (!(inlineContent instanceof HTMLElement)) return null;

  const ownerDocument = inlineContent.ownerDocument;
  const range = ownerDocument.createRange();
  range.selectNodeContents(inlineContent);
  const firstLineRect = Array.from(range.getClientRects())
    .find((rect) => rect.width > 0 && rect.height > 0);
  const textRect = firstLineRect ?? range.getBoundingClientRect();
  range.detach();

  if (textRect.height > 0) return textRect;

  const fallbackRect = inlineContent.getBoundingClientRect();
  return fallbackRect.height > 0 ? fallbackRect : null;
}

function alignSideMenuWithBlockText(editorElement: HTMLElement, blockId: string): boolean {
  const blockElement = blockElementById(editorElement, blockId);
  const sideMenu = sideMenuElementForEditor(editorElement);
  if (!blockElement || !sideMenu) return false;

  const anchorRect = blockTextAnchorRect(blockElement);
  if (!anchorRect) return false;

  sideMenu.style.removeProperty("translate");
  const sideMenuRect = sideMenu.getBoundingClientRect();
  if (sideMenuRect.height <= 0) return false;

  const anchorCenter = anchorRect.top + anchorRect.height / 2;
  const sideMenuCenter = sideMenuRect.top + sideMenuRect.height / 2;
  sideMenu.style.setProperty("translate", `0 ${anchorCenter - sideMenuCenter}px`);
  return true;
}

function createSideMenuAlignmentState(): SideMenuAlignmentState {
  return {
    attemptsRemaining: SIDE_MENU_ALIGNMENT_ATTEMPTS,
    frame: null,
    hasObservedTargets: false,
  };
}

function createSideMenuResizeObserver(onResize: () => void): ResizeObserver | null {
  return typeof ResizeObserver === "undefined"
    ? null
    : new ResizeObserver(onResize);
}

function observeSideMenuAlignmentTargets({
  blockId,
  editorElement,
  resizeObserver,
  state,
}: {
  blockId: string;
  editorElement: HTMLElement;
  resizeObserver: ResizeObserver | null;
  state: SideMenuAlignmentState;
}) {
  if (state.hasObservedTargets) return;

  const blockElement = blockElementById(editorElement, blockId);
  const sideMenu = sideMenuElementForEditor(editorElement);
  if (!resizeObserver || !blockElement || !sideMenu) return;

  resizeObserver.observe(blockElement);
  resizeObserver.observe(sideMenu);
  state.hasObservedTargets = true;
}

function scheduleSideMenuTextAlignment(context: SideMenuAlignmentContext) {
  const { blockId, editorElement, observeTargets, ownerWindow, retry, state } = context;
  if (state.frame !== null) return;

  state.frame = ownerWindow.requestAnimationFrame(() => {
    state.frame = null;
    const aligned = alignSideMenuWithBlockText(editorElement, blockId);
    observeTargets();
    if (!aligned && state.attemptsRemaining > 0) {
      state.attemptsRemaining -= 1;
      retry();
    }
  });
}

function createSideMenuAlignmentCleanup({
  editorElement,
  ownerWindow,
  resizeObserver,
  scheduleAlignment,
  state,
}: {
  editorElement: HTMLElement;
  ownerWindow: Window;
  resizeObserver: ResizeObserver | null;
  scheduleAlignment: () => void;
  state: SideMenuAlignmentState;
}) {
  return () => {
    if (state.frame !== null) ownerWindow.cancelAnimationFrame(state.frame);
    resizeObserver?.disconnect();
    ownerWindow.removeEventListener("resize", scheduleAlignment);
    sideMenuElementForEditor(editorElement)?.style.removeProperty("translate");
  };
}

function createSideMenuAlignmentController(editor: BlockNoteEditor, blockId: string) {
  const editorElement = editorBlockElement(editor);
  const ownerWindow = editorElement?.ownerDocument.defaultView;
  if (!editorElement || !ownerWindow) return undefined;

  const state = createSideMenuAlignmentState();
  let resizeObserver: ResizeObserver | null = null;
  const observeTargets = () => observeSideMenuAlignmentTargets({
    blockId,
    editorElement,
    resizeObserver,
    state,
  });
  const scheduleAlignment = () => scheduleSideMenuTextAlignment({
    blockId,
    editorElement,
    observeTargets,
    ownerWindow,
    retry: scheduleAlignment,
    state,
  });

  resizeObserver = createSideMenuResizeObserver(scheduleAlignment);
  scheduleAlignment();
  observeTargets();
  ownerWindow.addEventListener("resize", scheduleAlignment);

  return createSideMenuAlignmentCleanup({
    editorElement,
    ownerWindow,
    resizeObserver,
    scheduleAlignment,
    state,
  });
}

function useSideMenuTextAlignment(editor: BlockNoteEditor, block: PlainEditorBlock | undefined) {
  const blockId = block?.id;

  useLayoutEffect(() => {
    if (!blockId) return;

    return createSideMenuAlignmentController(editor, blockId);
  }, [blockId, editor]);
}

function usePlainSideMenuBlock() {
  const editor = useBlockNoteEditor();
  const block = useExtensionState(SideMenuExtension, {
    editor,
    selector: (state) => state?.block as PlainEditorBlock | undefined,
  });
  return { block, editor };
}

function PlainEditorDragHandleButton({
  children,
  dragHandleMenu,
}: SideMenuProps & { children?: ReactNode }) {
  const Components = useComponentsContext()!;
  const dict = useDictionary();
  const sideMenu = useExtension(SideMenuExtension);
  const { block, editor } = usePlainSideMenuBlock();
  const MenuComponent: ComponentType<{ children?: ReactNode }> =
    dragHandleMenu ?? DragHandleMenu;
  const reorderStateRef = useRef<PointerReorderState | null>(null);
  const suppressNextClickRef = useRef(false);

  const clearReorderState = useCallback(() => {
    const state = reorderStateRef.current;
    if (state) {
      state.clearListeners();
      cleanupReorderAffordances(state.affordances);
    }
    reorderStateRef.current = null;
  }, []);

  const finishPointerReorder = useCallback((event: PointerEvent) => {
    const state = reorderStateRef.current;
    if (!state || event.pointerId !== state.pointerId) return;

    clearReorderState();
    if (!state.hasMoved) return;

    event.preventDefault();
    suppressNextClickRef.current = true;
    const dropTarget = state.lastDropTarget ?? validDropTarget({
      editor,
      state,
      x: event.clientX,
      y: event.clientY,
    });
    if (!dropTarget) return;

    const moved = moveBlockByPointerDrop({
      editor,
      draggedBlockId: state.draggedBlockId,
      targetBlockId: dropTarget.blockId,
      placement: dropTarget.placement,
    });
    if (!moved) suppressNextClickRef.current = false;
  }, [clearReorderState, editor]);

  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    if ((typeof event.button === "number" && event.button !== 0) || event.isPrimary === false) {
      return;
    }
    if (!block) {
      event.preventDefault();
      return;
    }

    const editorElement = editorBlockElement(editor);
    const liveDraggedBlock = liveBlock(editor, block.id);
    if (!editorElement || !liveDraggedBlock) {
      event.preventDefault();
      return;
    }

    clearReorderState();
    const ownerDocument = event.currentTarget.ownerDocument;
    const pointerId = event.pointerId;
    const handlePointerMove = (nativeEvent: PointerEvent) => {
      const state = reorderStateRef.current;
      if (!state || nativeEvent.pointerId !== state.pointerId) return;

      const distance = Math.hypot(
        nativeEvent.clientX - state.startX,
        nativeEvent.clientY - state.startY,
      );
      if (!state.hasMoved && distance < POINTER_REORDER_THRESHOLD_PX) return;

      state.hasMoved = true;
      suppressNextClickRef.current = true;
      state.affordances ??= createReorderAffordances(state);
      if (!state.affordances) return;

      updateDragPreview(state.affordances, nativeEvent.clientX, nativeEvent.clientY);
      state.lastDropTarget = validDropTarget({
        editor,
        state,
        x: nativeEvent.clientX,
        y: nativeEvent.clientY,
      });
      updateDropIndicator(state.affordances, state.lastDropTarget ?? null);
      nativeEvent.preventDefault();
    };
    const handlePointerUp = (nativeEvent: PointerEvent) => finishPointerReorder(nativeEvent);
    const handlePointerCancel = (nativeEvent: PointerEvent) => {
      if (nativeEvent.pointerId !== pointerId) return;
      clearReorderState();
    };

    ownerDocument.addEventListener("pointermove", handlePointerMove, true);
    ownerDocument.addEventListener("pointerup", handlePointerUp, true);
    ownerDocument.addEventListener("pointercancel", handlePointerCancel, true);
    reorderStateRef.current = {
      clearListeners: () => {
        ownerDocument.removeEventListener("pointermove", handlePointerMove, true);
        ownerDocument.removeEventListener("pointerup", handlePointerUp, true);
        ownerDocument.removeEventListener("pointercancel", handlePointerCancel, true);
      },
      draggedBlockId: liveDraggedBlock.id,
      editorElement,
      hasMoved: false,
      ownerDocument,
      pointerId,
      startX: event.clientX,
      startY: event.clientY,
    };

    try {
      event.currentTarget.setPointerCapture?.(pointerId);
    } catch {
      // Document-level pointer listeners still complete the reorder gesture.
    }
  }, [block, clearReorderState, editor, finishPointerReorder]);

  const onClickCapture = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    if (!suppressNextClickRef.current) return;
    suppressNextClickRef.current = false;
    event.preventDefault();
    event.stopPropagation();
  }, []);

  if (!block) return null;

  return (
    <Components.Generic.Menu.Root
      onOpenChange={(open: boolean) => {
        if (open) sideMenu.freezeMenu();
        else sideMenu.unfreezeMenu();
      }}
      position="left"
    >
      <Components.Generic.Menu.Trigger>
        <span
          className="sessio-plain-editor-drag-handle"
          onPointerDown={onPointerDown}
          onClickCapture={onClickCapture}
        >
          <Components.SideMenu.Button
            label={dict.side_menu.drag_handle_label}
            draggable={false}
            onDragStart={(event) => event.preventDefault()}
            onDragEnd={sideMenu.blockDragEnd}
            className="bn-button"
            icon={<GripVertical aria-hidden="true" size={18} data-test="dragHandle" />}
          />
        </span>
      </Components.Generic.Menu.Trigger>
      <MenuComponent>{children}</MenuComponent>
    </Components.Generic.Menu.Root>
  );
}

function PlainEditorSideMenu(props: SideMenuProps) {
  const { block, editor } = usePlainSideMenuBlock();
  useSideMenuTextAlignment(editor, block);

  return (
    <SideMenu {...props}>
      <AddBlockButton />
      <PlainEditorDragHandleButton dragHandleMenu={PlainEditorDragHandleMenu} />
    </SideMenu>
  );
}

export function PlainEditorSideMenuController() {
  return <SideMenuController sideMenu={PlainEditorSideMenu} />;
}
