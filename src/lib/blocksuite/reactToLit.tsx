import { useCallback, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { html, LitElement, type TemplateResult } from "lit";

type PortalListener = (target: LitReactPortal) => void;

type ElementOrFactory = ReactNode | (() => ReactNode);

function randomId() {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `portal-${Math.random().toString(36).slice(2, 10)}`;
}

const LIT_REACT_PORTAL = "sessio-lit-react-portal";

function createLitPortalAnchor(
  portalId: string,
  elementOrFactory: ElementOrFactory,
  shouldRerender: boolean,
  notify: PortalListener,
) {
  return html`<sessio-lit-react-portal
    .portalId=${portalId}
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

  private reactRoot: Root | null = null;

  private lastRenderedFactory: ElementOrFactory | null = null;

  constructor() {
    super();
    this.portalId = "";
    this.elementOrFactory = null;
    this.shouldRerender = false;
    this.notify = undefined;
  }

  override connectedCallback() {
    super.connectedCallback();
    this.notify?.(this);
    this.renderReactPortal();
  }

  override createRenderRoot() {
    return this;
  }

  override updated() {
    this.renderReactPortal();
  }

  override disconnectedCallback() {
    this.teardownReactPortal();
    super.disconnectedCallback();
  }

  renderReactPortal() {
    const nextFactory = this.elementOrFactory;
    if (!nextFactory) return;
    if (!this.shouldRerender && this.reactRoot && this.lastRenderedFactory) {
      return;
    }
    this.reactRoot ??= createRoot(this);
    this.lastRenderedFactory = nextFactory;
    const element =
      typeof nextFactory === "function"
        ? nextFactory()
        : nextFactory;
    this.reactRoot.render(element);
  }

  teardownReactPortal() {
    this.lastRenderedFactory = null;
    this.reactRoot?.unmount();
    this.reactRoot = null;
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
  const reactToLit = useCallback<ReactToLit>((elementOrFactory, rerendering = false) => {
    const portalId = randomId();
    return createLitPortalAnchor(
      portalId,
      elementOrFactory,
      rerendering,
      (target) => {
        target.renderReactPortal();
      },
    );
  }, []);

  return [reactToLit, []] as const;
}
