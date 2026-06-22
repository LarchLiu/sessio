import type { Constructor } from '@blocksuite/global/utils';
import type { CSSResultGroup, CSSResultOrNative } from 'lit';
import { CSSResult, LitElement } from 'lit';

import { BLOCKSUITE_STYLE_SCOPE_CLASS } from './consts.js';

const BLOCKSUITE_STYLE_SCOPE_SELECTOR = `.${BLOCKSUITE_STYLE_SCOPE_CLASS}`;
const UNSCOPED_AT_RULES = new Set([
  'counter-style',
  'font-face',
  'font-feature-values',
  'keyframes',
  'page',
  'property',
]);

function addScopeClassToSimpleSelector(selector: string) {
  if (/^[.#]?[a-zA-Z_][\w-]*$/.test(selector)) {
    return `${selector}${BLOCKSUITE_STYLE_SCOPE_SELECTOR}`;
  }
  return null;
}

function findMatchingBrace(cssText: string, start: number) {
  let depth = 0;
  let quote: '"' | "'" | null = null;
  let escaped = false;
  let inComment = false;

  for (let index = start; index < cssText.length; index++) {
    const char = cssText[index];
    const nextChar = cssText[index + 1];

    if (inComment) {
      if (char === '*' && nextChar === '/') {
        inComment = false;
        index++;
      }
      continue;
    }

    if (quote) {
      if (escaped) {
        escaped = false;
      } else if (char === '\\') {
        escaped = true;
      } else if (char === quote) {
        quote = null;
      }
      continue;
    }

    if (char === '/' && nextChar === '*') {
      inComment = true;
      index++;
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }

    if (char === '{') {
      depth++;
      continue;
    }

    if (char === '}') {
      depth--;
      if (depth === 0) {
        return index;
      }
    }
  }

  return -1;
}

function findTopLevelToken(cssText: string, start: number) {
  let quote: '"' | "'" | null = null;
  let escaped = false;
  let inComment = false;
  let parenDepth = 0;
  let bracketDepth = 0;

  for (let index = start; index < cssText.length; index++) {
    const char = cssText[index];
    const nextChar = cssText[index + 1];

    if (inComment) {
      if (char === '*' && nextChar === '/') {
        inComment = false;
        index++;
      }
      continue;
    }

    if (quote) {
      if (escaped) {
        escaped = false;
      } else if (char === '\\') {
        escaped = true;
      } else if (char === quote) {
        quote = null;
      }
      continue;
    }

    if (char === '/' && nextChar === '*') {
      inComment = true;
      index++;
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }

    if (char === '(') {
      parenDepth++;
      continue;
    }

    if (char === ')') {
      parenDepth = Math.max(0, parenDepth - 1);
      continue;
    }

    if (char === '[') {
      bracketDepth++;
      continue;
    }

    if (char === ']') {
      bracketDepth = Math.max(0, bracketDepth - 1);
      continue;
    }

    if (parenDepth === 0 && bracketDepth === 0 && (char === '{' || char === ';')) {
      return { char, index };
    }
  }

  return null;
}

function splitSelectorList(selectorText: string) {
  const selectors: string[] = [];
  let start = 0;
  let quote: '"' | "'" | null = null;
  let escaped = false;
  let parenDepth = 0;
  let bracketDepth = 0;

  for (let index = 0; index < selectorText.length; index++) {
    const char = selectorText[index];

    if (quote) {
      if (escaped) {
        escaped = false;
      } else if (char === '\\') {
        escaped = true;
      } else if (char === quote) {
        quote = null;
      }
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }

    if (char === '(') {
      parenDepth++;
      continue;
    }

    if (char === ')') {
      parenDepth = Math.max(0, parenDepth - 1);
      continue;
    }

    if (char === '[') {
      bracketDepth++;
      continue;
    }

    if (char === ']') {
      bracketDepth = Math.max(0, bracketDepth - 1);
      continue;
    }

    if (char === ',' && parenDepth === 0 && bracketDepth === 0) {
      selectors.push(selectorText.slice(start, index));
      start = index + 1;
    }
  }

  selectors.push(selectorText.slice(start));
  return selectors;
}

function scopeSelector(selector: string) {
  const trimmed = selector.trim();
  if (!trimmed || trimmed.includes(BLOCKSUITE_STYLE_SCOPE_CLASS)) {
    return trimmed;
  }

  const hostScoped = trimmed
    .replace(/:host\(([^)]*)\)/g, `${BLOCKSUITE_STYLE_SCOPE_SELECTOR}$1`)
    .replace(/:host\b/g, BLOCKSUITE_STYLE_SCOPE_SELECTOR);
  if (hostScoped !== trimmed) {
    return hostScoped;
  }

  if (/^(html|body|:root)(?=$|[\s>+~.#:[,])/.test(trimmed)) {
    return trimmed.replace(/^(html|body|:root)/, BLOCKSUITE_STYLE_SCOPE_SELECTOR);
  }

  const scoped = `${BLOCKSUITE_STYLE_SCOPE_SELECTOR} ${trimmed}`;
  const selfScoped = addScopeClassToSimpleSelector(trimmed);
  return selfScoped ? `${scoped}, ${selfScoped}` : scoped;
}

function scopeSelectorList(selectorText: string) {
  return splitSelectorList(selectorText).map(scopeSelector).join(', ');
}

function getAtRuleName(prelude: string) {
  return prelude.slice(1).match(/^[\w-]+/)?.[0].toLowerCase() ?? '';
}

function scopeCssTextFallback(cssText: string): string {
  let output = '';
  let cursor = 0;

  while (cursor < cssText.length) {
    const token = findTopLevelToken(cssText, cursor);
    if (!token) {
      output += cssText.slice(cursor);
      break;
    }

    if (token.char === ';') {
      output += cssText.slice(cursor, token.index + 1);
      cursor = token.index + 1;
      continue;
    }

    const blockEnd = findMatchingBrace(cssText, token.index);
    if (blockEnd === -1) {
      output += cssText.slice(cursor);
      break;
    }

    const prelude = cssText.slice(cursor, token.index);
    const body = cssText.slice(token.index + 1, blockEnd);
    const trimmedPrelude = prelude.trim();

    if (trimmedPrelude.startsWith('@')) {
      const atRuleName = getAtRuleName(trimmedPrelude);
      const nextBody = UNSCOPED_AT_RULES.has(atRuleName)
        ? body
        : scopeCssTextFallback(body);
      output += `${prelude}{${nextBody}}`;
    } else {
      const leadingWhitespace = prelude.match(/^\s*/)?.[0] ?? '';
      output += `${leadingWhitespace}${scopeSelectorList(trimmedPrelude)}{${body}}`;
    }

    cursor = blockEnd + 1;
  }

  return output;
}

function scopeCssText(cssText: string) {
  try {
    const styleSheet = new CSSStyleSheet();
    styleSheet.replaceSync(cssText);

    for (const rule of Array.from(styleSheet.cssRules)) {
      scopeCssRule(rule);
    }

    return Array.from(styleSheet.cssRules)
      .map(rule => rule.cssText)
      .join('\n');
  } catch {
    return scopeCssTextFallback(cssText);
  }
}

function scopeCssRule(rule: CSSRule) {
  if (rule instanceof CSSStyleRule) {
    rule.selectorText = scopeSelectorList(rule.selectorText);
    return;
  }

  if (
    hasNestedCssRules(rule) &&
    rule.type !== CSSRule.KEYFRAMES_RULE &&
    rule.type !== CSSRule.FONT_FACE_RULE
  ) {
    for (const childRule of Array.from(rule.cssRules)) {
      scopeCssRule(childRule);
    }
  }
}

function hasNestedCssRules(rule: CSSRule): rule is CSSRule & { cssRules: CSSRuleList } {
  return 'cssRules' in rule && rule.cssRules instanceof CSSRuleList;
}

export class ShadowlessElement extends LitElement {
  // Map of the number of styles injected into a node
  // A reference count of the number of ShadowlessElements that are still connected
  static connectedCount = new WeakMap<
    Constructor, // class
    WeakMap<Node, number>
  >();

  static onDisconnectedMap = new WeakMap<
    Constructor, // class
    WeakMap<Node, (() => void) | null>
  >();

  // styles registered in ShadowlessElement will be available globally
  // even if the element is not being rendered
  protected static override finalizeStyles(
    styles?: CSSResultGroup
  ): CSSResultOrNative[] {
    const elementStyles = super.finalizeStyles(styles);
    // XXX: This breaks component encapsulation and applies styles to the document.
    // These styles should be manually scoped.
    elementStyles.forEach((s: CSSResultOrNative) => {
      if (s instanceof CSSResult && typeof document !== 'undefined') {
        const styleRoot = document.head;
        const style = document.createElement('style');
        style.textContent = scopeCssText(s.cssText);
        styleRoot.append(style);
      }
    });
    return elementStyles;
  }

  private getConnectedCount() {
    const SE = this.constructor as typeof ShadowlessElement;
    return SE.connectedCount.get(SE)?.get(this.getRootNode()) ?? 0;
  }

  private setConnectedCount(count: number) {
    const SE = this.constructor as typeof ShadowlessElement;

    if (!SE.connectedCount.has(SE)) {
      SE.connectedCount.set(SE, new WeakMap());
    }

    SE.connectedCount.get(SE)?.set(this.getRootNode(), count);
  }

  override connectedCallback(): void {
    super.connectedCallback();
    const parentRoot = this.getRootNode();
    const SE = this.constructor as typeof ShadowlessElement;
    const insideShadowRoot = parentRoot instanceof ShadowRoot;
    const styleInjectedCount = this.getConnectedCount();

    if (
      !insideShadowRoot &&
      !this.closest(BLOCKSUITE_STYLE_SCOPE_SELECTOR)
    ) {
      this.classList.add(BLOCKSUITE_STYLE_SCOPE_CLASS);
    }

    if (styleInjectedCount === 0 && insideShadowRoot) {
      const elementStyles = SE.elementStyles;
      const injectedStyles: HTMLStyleElement[] = [];
      elementStyles.forEach((s: CSSResultOrNative) => {
        if (s instanceof CSSResult && typeof document !== 'undefined') {
          const style = document.createElement('style');
          style.textContent = s.cssText;
          parentRoot.prepend(style);
          injectedStyles.push(style);
        }
      });
      if (!SE.onDisconnectedMap.has(SE)) {
        SE.onDisconnectedMap.set(SE, new WeakMap());
      }
      SE.onDisconnectedMap.get(SE)?.set(parentRoot, () => {
        injectedStyles.forEach(style => style.remove());
      });
    }
    this.setConnectedCount(styleInjectedCount + 1);
  }

  override createRenderRoot() {
    return this;
  }

  override disconnectedCallback(): void {
    const parentRoot = this.getRootNode();
    super.disconnectedCallback();
    const SE = this.constructor as typeof ShadowlessElement;
    let styleInjectedCount = this.getConnectedCount();
    styleInjectedCount--;
    this.setConnectedCount(styleInjectedCount);

    if (styleInjectedCount === 0) {
      // remove the style element when the last shadowless element is disconnected in the parent root
      SE.onDisconnectedMap.get(SE)?.get(parentRoot)?.();
    }
  }
}
