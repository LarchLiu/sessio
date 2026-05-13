/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        sans: [
          "-apple-system",
          "BlinkMacSystemFont",
          "SF Pro Text",
          "Segoe UI",
          "Roboto",
          "Helvetica Neue",
          "sans-serif",
        ],
        mono: ["SF Mono", "Menlo", "Monaco", "Consolas", "monospace"],
      },
      fontSize: {
        caption: ["11px", { lineHeight: "1.35", letterSpacing: "0.04em" }],
        meta: ["12px", { lineHeight: "1.45" }],
        "body-sm": ["13px", { lineHeight: "1.55" }],
        body: ["14px", { lineHeight: "1.55" }],
        subtitle: ["16px", { lineHeight: "1.4" }],
        title: ["17px", { lineHeight: "1.3" }],
        hero: ["20px", { lineHeight: "1.2" }],
      },
      colors: {
        ink: "rgb(var(--color-fg) / <alpha-value>)",
        surface: {
          DEFAULT: "rgb(var(--color-bg) / <alpha-value>)",
          sidebar: "rgb(var(--color-bg-sidebar) / <alpha-value>)",
          panel: "rgb(var(--color-bg-panel) / <alpha-value>)",
          "panel-alt": "rgb(var(--color-bg-panel-alt) / <alpha-value>)",
        },
        tooltip: {
          bg: "rgb(var(--color-tooltip-bg) / <alpha-value>)",
          fg: "rgb(var(--color-tooltip-fg) / <alpha-value>)",
        },
        "accent-purple": "rgb(var(--color-accent-purple) / <alpha-value>)",
        status: {
          warn: "rgb(var(--color-status-warn) / <alpha-value>)",
          error: "rgb(var(--color-status-error) / <alpha-value>)",
        },
      },
    },
  },
  plugins: [],
};
