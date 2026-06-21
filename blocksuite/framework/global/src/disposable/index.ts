import { Subject, Subscription } from 'rxjs';

type DisposeCallback = () => void;

export interface Disposable {
  dispose: DisposeCallback;
}

export type DisposableMember =
  | Disposable
  | Subscription
  | Subject<any>
  | DisposeCallback;

export class DisposableGroup {
  private _disposables: DisposableMember[] = [];

  private _disposed = false;

  get disposed() {
    return this._disposed;
  }

  /**
   * Add to group to be disposed with others.
   * This will be immediately disposed if this group has already been disposed.
   */
  add(d: DisposableMember) {
    if (this._disposed) {
      disposeMember(d);
      return;
    }
    this._disposables.push(d);
  }

  addFromEvent<N extends keyof WindowEventMap>(
    element: Window | null | undefined,
    eventName: N,
    handler: (e: WindowEventMap[N]) => void,
    options?: boolean | AddEventListenerOptions
  ): void;
  addFromEvent<N extends keyof DocumentEventMap>(
    element: Document | null | undefined,
    eventName: N,
    handler: (e: DocumentEventMap[N]) => void,
    eventOptions?: boolean | AddEventListenerOptions
  ): void;
  addFromEvent<N extends keyof HTMLElementEventMap>(
    element: HTMLElement | null | undefined,
    eventName: N,
    handler: (e: HTMLElementEventMap[N]) => void,
    eventOptions?: boolean | AddEventListenerOptions
  ): void;
  addFromEvent<N extends keyof VisualViewportEventMap>(
    element: VisualViewport | null | undefined,
    eventName: N,
    handler: (e: VisualViewportEventMap[N]) => void,
    eventOptions?: boolean | AddEventListenerOptions
  ): void;
  addFromEvent<N extends keyof VirtualKeyboardEventMap>(
    element: VirtualKeyboard | null | undefined,
    eventName: N,
    handler: (e: VirtualKeyboardEventMap[N]) => void,
    eventOptions?: boolean | AddEventListenerOptions
  ): void;
  addFromEvent<E extends Event = Event>(
    target: EventTarget | null | undefined,
    type: string,
    handler: (e: E) => void,
    eventOptions?: boolean | AddEventListenerOptions
  ): void;

  addFromEvent(
    target:
      | EventTarget
      | HTMLElement
      | Window
      | Document
      | VisualViewport
      | VirtualKeyboard
      | null
      | undefined,
    type: string,
    handler: (e: Event) => void,
    eventOptions?: boolean | AddEventListenerOptions
  ) {
    if (!target) {
      return;
    }
    this.add({
      dispose: () => {
        target.removeEventListener(type, handler as () => void, eventOptions);
      },
    });
    target.addEventListener(type, handler as () => void, eventOptions);
  }

  dispose() {
    disposeAll(this._disposables);
    this._disposables = [];
    this._disposed = true;
  }
}

export function disposeMember(disposable: DisposableMember) {
  try {
    if (disposable instanceof Subscription) {
      disposable.unsubscribe();
    } else if (disposable instanceof Subject) {
      disposable.complete();
    } else if (typeof disposable === 'function') {
      disposable();
    } else {
      disposable.dispose();
    }
  } catch (e) {
    console.error(e);
  }
}

function disposeAll(disposables: DisposableMember[]) {
  disposables.forEach(disposeMember);
}
