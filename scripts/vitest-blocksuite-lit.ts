import { vi } from "vitest";

if (typeof window !== "undefined") {
  const registry = window.customElements;

  if (
    !registry ||
    typeof registry.define !== "function" ||
    typeof registry.get !== "function" ||
    typeof registry.whenDefined !== "function"
  ) {
    const customElementsRegistry = {
      define: vi.fn(),
      get: vi.fn(),
      whenDefined: vi.fn(),
      upgrade: vi.fn(),
    };
    vi.stubGlobal("customElements", customElementsRegistry);
  }
}
