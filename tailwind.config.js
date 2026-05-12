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
      // Semantic font sizes — change here and it propagates everywhere.
      // Use these instead of arbitrary `text-[12px]` values.
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
        ink: {
          50: "#f7f7f8",
          100: "#eeeef0",
          200: "#d9d9de",
          300: "#b6b6bd",
          400: "#8c8c95",
          500: "#6b6b75",
          600: "#4f4f58",
          700: "#3a3a42",
          800: "#26262c",
          900: "#16161a",
        },
      },
    },
  },
  plugins: [],
};
