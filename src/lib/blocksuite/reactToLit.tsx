import { useCallback, useState, type ReactNode, type ReactPortal } from "react";
import { createPortal } from "react-dom";
import { html, LitElement, type TemplateResult } from "lit";
import { customElement, property } from "lit/decorators.js";

type PortalEvent = {
  name: "connectedCallback" | "disconnectedCallback" | "willUpdate";
  target: LitReactPortal;
};

type PortalListener = (event: PortalEvent) => void;

function randomId() {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `portal-${Math.random().toString(36).slice(2, 10)}`;
}

const LIT_REACT_PORTAL = "sessio-lit-react-portal";

function createLitPortalAnchor(callback: PortalListener) {
  return html`<sessio-lit-react-portal
    .notify=${callback}
    portalId=${randomId()}
  ></sessio-lit-react-portal>`;
}

@customElement(LIT_REACT_PORTAL)
class LitReactPortal extends LitElement {
  @property({ type: String })
  accessor portalId = "";

  @property({ attribute: false })
  accessor notify: PortalListener | undefined = undefined;

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

  override disconnectedCallback() {
    super.disconnectedCallback();
    this.notify?.({
      name: "disconnectedCallback",
      target: this,
    });
  }
}

declare global {
  interface HTMLElementTagNameMap {
    "sessio-lit-react-portal": LitReactPortal;
  }
}

type ElementOrFactory = ReactNode | (() => ReactNode);

type LitPortal = {
  id: string;
  portal: ReactPortal;
  litElement: LitReactPortal;
};

export type ReactToLit = (
  elementOrFactory: ElementOrFactory,
  rerendering?: boolean,
) => TemplateResult;

export function useReactToLitBridge() {
  const [portals, setPortals] = useState<LitPortal[]>([]);

  const reactToLit = useCallback<ReactToLit>((elementOrFactory, rerendering = false) => {
    const element =
      typeof elementOrFactory === "function" ? elementOrFactory() : elementOrFactory;

    return createLitPortalAnchor((event) => {
      setPortals((current) => {
        const { name, target } = event;
        const id = target.portalId;
        let next = current;

        const updatePortals = () => {
          let oldIndex = current.findIndex((item) => item.litElement === target);
          oldIndex = oldIndex === -1 ? current.length : oldIndex;
          next = [
            ...current.slice(0, oldIndex),
            {
              id,
              portal: createPortal(element, target),
              litElement: target,
            },
            ...current.slice(oldIndex + 1),
          ];
        };

        switch (name) {
          case "connectedCallback":
            updatePortals();
            break;
          case "disconnectedCallback":
            next = current.filter((item) => item.litElement.isConnected);
            break;
          case "willUpdate":
            if (!target.isConnected || !rerendering) break;
            updatePortals();
            break;
        }

        return next;
      });
    });
  }, []);

  return [reactToLit, portals] as const;
}
