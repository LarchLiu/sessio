import { html, LitElement, type TemplateResult } from "lit";
import { useCallback, useState, type ReactNode, type ReactPortal } from "react";
import ReactDOM from "react-dom";

type PortalEvent = {
  name: "connectedCallback" | "disconnectedCallback" | "willUpdate";
  target: LitReactPortal;
};

type PortalListener = (event: PortalEvent) => void;

type ElementOrFactory = ReactNode | (() => ReactNode);

type LitPortalEntry = {
  id: string;
  portal: ReactPortal;
  litElement: LitReactPortal;
};

function randomId() {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `portal-${Math.random().toString(36).slice(2, 10)}`;
}

const LIT_REACT_PORTAL = "sessio-lit-react-portal";

function createLitPortalAnchor(
  elementOrFactory: ElementOrFactory,
  shouldRerender: boolean,
  notify: PortalListener,
) {
  return html`<sessio-lit-react-portal
    .portalId=${randomId()}
    .elementOrFactory=${elementOrFactory}
    .shouldRerender=${shouldRerender}
    .notify=${notify}
  ></sessio-lit-react-portal>`;
}

class LitReactPortal extends LitElement {
  static properties = {
    portalId: { type: String },
    elementOrFactory: { attribute: false },
    shouldRerender: { attribute: false },
    notify: { attribute: false },
  };

  declare portalId: string;

  declare elementOrFactory: ElementOrFactory | null;

  declare shouldRerender: boolean;

  declare notify: PortalListener | undefined;

  constructor() {
    super();
    this.portalId = "";
    this.elementOrFactory = null;
    this.shouldRerender = false;
    this.notify = undefined;
  }

  override connectedCallback() {
    super.connectedCallback();
    this.notify?.({
      name: "connectedCallback",
      target: this,
    });
  }

  override attributeChangedCallback(name: string, oldVal: string | null, newVal: string | null) {
    super.attributeChangedCallback(name, oldVal, newVal);
    if (name.toLowerCase() === "portalid") {
      this.notify?.({
        name: "willUpdate",
        target: this,
      });
    }
  }

  override createRenderRoot() {
    return this;
  }

  override updated() {
    this.notify?.({
      name: "willUpdate",
      target: this,
    });
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    this.notify?.({
      name: "disconnectedCallback",
      target: this,
    });
  }
}

if (!customElements.get(LIT_REACT_PORTAL)) {
  customElements.define(LIT_REACT_PORTAL, LitReactPortal);
}

declare global {
  interface HTMLElementTagNameMap {
    "sessio-lit-react-portal": LitReactPortal;
  }
}

export type ReactToLit = (
  elementOrFactory: ElementOrFactory,
  rerendering?: boolean,
) => TemplateResult;

export function useReactToLitBridge() {
  const [portals, setPortals] = useState<LitPortalEntry[]>([]);

  const reactToLit = useCallback<ReactToLit>((elementOrFactory, rerendering = false) => {
    return createLitPortalAnchor(
      elementOrFactory,
      rerendering,
      (event) => {
        setPortals((currentPortals) => {
          const { name, target } = event;
          const id = target.portalId;
          const nextElement = target.elementOrFactory;
          let nextPortals = currentPortals;

          const updatePortals = () => {
            if (!nextElement) {
              return;
            }

            const element =
              typeof nextElement === "function"
                ? nextElement()
                : nextElement;
            const existingIndex = currentPortals.findIndex(
              (entry) => entry.litElement === target,
            );
            const insertIndex = existingIndex === -1 ? currentPortals.length : existingIndex;

            nextPortals = currentPortals.toSpliced(insertIndex, 1, {
              id,
              portal: ReactDOM.createPortal(element, target),
              litElement: target,
            });
          };

          switch (name) {
            case "connectedCallback":
              updatePortals();
              break;
            case "disconnectedCallback":
              nextPortals = currentPortals.filter((entry) => entry.litElement.isConnected);
              break;
            case "willUpdate":
              if (!target.isConnected || !rerendering) {
                break;
              }
              updatePortals();
              break;
          }

          return nextPortals;
        });
      },
    );
  }, []);

  return [reactToLit, portals] as const;
}
