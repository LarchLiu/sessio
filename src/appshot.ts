import { useEffect } from "react";
import type { ComposerAttachmentDraft } from "./components/ComposerAttachments";
import type { ChatComposerController } from "./hooks/useChatComposer";

type ActiveComposerEntry = {
  controller: ChatComposerController;
  active: boolean;
};

let activeComposer: ActiveComposerEntry | null = null;

export function setActiveAppshotComposer(
  controller: ChatComposerController,
  active: boolean,
): void {
  if (!active) {
    if (activeComposer?.controller === controller) {
      activeComposer = null;
    }
    return;
  }
  activeComposer = { controller, active };
}

export function clearActiveAppshotComposer(controller: ChatComposerController): void {
  if (activeComposer?.controller === controller) {
    activeComposer = null;
  }
}

export async function appendAppshotToActiveComposer(path: string): Promise<boolean> {
  const controller = activeComposer?.controller;
  if (!controller) return false;
  if (!controller.supportsAttachments || !controller.supportsImageAttachments) return false;
  const draft: ComposerAttachmentDraft = {
    kind: "image",
    path,
    mimeType: "image/png",
    name: "Appshot.png",
    displayName: "Appshot",
  };
  await controller.appendAttachments([draft]);
  window.requestAnimationFrame(() => controller.textareaRef.current?.focus());
  return true;
}

export function useAppshotComposerRegistration(
  composer: ChatComposerController,
  active = true,
): void {
  useEffect(() => {
    setActiveAppshotComposer(composer, active);
    return () => clearActiveAppshotComposer(composer);
  }, [active, composer]);
}
