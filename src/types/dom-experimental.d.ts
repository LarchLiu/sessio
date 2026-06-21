interface VirtualKeyboardEventMap {
  geometrychange: Event;
}

interface VirtualKeyboard extends EventTarget {
  overlaysContent: boolean;
  boundingRect: DOMRectReadOnly;
  show(): Promise<void>;
  hide(): Promise<void>;
}
