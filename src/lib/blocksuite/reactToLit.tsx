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

type RerenderStrategy = "never" | "always" | "token";

function randomId() {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `portal-${Math.random().toString(36).slice(2, 10)}`;
}

const LIT_REACT_PORTAL = "sessio-lit-react-portal";

function createLitPortalAnchor(
  elementOrFactory: ElementOrFactory,
  rerenderStrategy: RerenderStrategy,
  rerenderToken: number | string | null,
  notify: PortalListener,
) {
  return html`<sessio-lit-react-portal
    .elementOrFactory=${elementOrFactory}
    .rerenderStrategy=${rerenderStrategy}
    .rerenderToken=${rerenderToken}
    .notify=${notify}
  ></sessio-lit-react-portal>`;
}

class LitReactPortal extends LitElement {
  static properties = {
    elementOrFactory: { attribute: false },
    rerenderStrategy: { attribute: false },
    rerenderToken: { attribute: false },
    notify: { attribute: false },
  };

  declare portalId: string;

  declare elementOrFactory: ElementOrFactory | null;

  declare rerenderStrategy: RerenderStrategy;

  declare rerenderToken: number | string | null;

  declare notify: PortalListener | undefined;

  constructor() {
    super();
    this.portalId = randomId();
    this.elementOrFactory = null;
    this.rerenderStrategy = "never";
    this.rerenderToken = null;
    this.notify = undefined;
  }

  override connectedCallback() {
    super.connectedCallback();
    this.notify?.({
      name: "connectedCallback",
      target: this,
    });
  }

  override createRenderRoot() {
    return this;
  }

  override updated(changedProperties: Map<string, unknown>) {
    const shouldNotify =
      this.rerenderStrategy === "always" ||
      (this.rerenderStrategy === "token" &&
        (changedProperties.has("rerenderStrategy") ||
          changedProperties.has("rerenderToken")));

    if (!shouldNotify) {
      return;
    }

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
  rerendering?: boolean | number | string | null,
) => TemplateResult;

export function useReactToLitBridge() {
  const [portals, setPortals] = useState<LitPortalEntry[]>([]);

  const reactToLit = useCallback<ReactToLit>((elementOrFactory, rerendering = false) => {
    const isTokenRerender =
      rerendering !== true &&
      rerendering !== false &&
      rerendering !== null &&
      rerendering !== undefined;
    const rerenderStrategy: RerenderStrategy =
      rerendering === true
        ? "always"
        : rerendering === false || rerendering === null || rerendering === undefined
          ? "never"
          : "token";
    const rerenderToken = isTokenRerender ? rerendering : null;

    return createLitPortalAnchor(
      elementOrFactory,
      rerenderStrategy,
      rerenderToken,
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
              if (
                !target.isConnected ||
                target.rerenderStrategy === "never"
              ) {
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
