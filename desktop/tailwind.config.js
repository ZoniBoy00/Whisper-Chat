/** @type {import('tailwindcss').Config} */
export default {
  darkMode: "class",
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        display: ['"Palatino Linotype"', "Palatino", '"Book Antiqua"', "Georgia", "serif"],
        sans: ["Candara", '"Segoe UI Variable"', '"Segoe UI"', "system-ui", "sans-serif"],
        mono: ['"Cascadia Mono"', '"Cascadia Code"', "Consolas", "monospace"],
      },
      colors: {
        wp: {
          bg: "rgb(var(--wp-bg) / <alpha-value>)",
          deep: "rgb(var(--wp-bg-deep) / <alpha-value>)",
          panel: "rgb(var(--wp-panel) / <alpha-value>)",
          "panel-2": "rgb(var(--wp-panel-2) / <alpha-value>)",
          "panel-3": "rgb(var(--wp-panel-3) / <alpha-value>)",
          line: "rgb(var(--wp-line) / <alpha-value>)",
          text: "rgb(var(--wp-text) / <alpha-value>)",
          dim: "rgb(var(--wp-text-dim) / <alpha-value>)",
          faint: "rgb(var(--wp-text-faint) / <alpha-value>)",
          accent: "rgb(var(--wp-accent) / <alpha-value>)",
          "accent-strong": "rgb(var(--wp-accent-strong) / <alpha-value>)",
          "accent-fg": "rgb(var(--wp-accent-fg) / <alpha-value>)",
          "bubble-out": "rgb(var(--wp-bubble-out) / <alpha-value>)",
          "bubble-out-2": "rgb(var(--wp-bubble-out-2) / <alpha-value>)",
          "bubble-in": "rgb(var(--wp-bubble-in) / <alpha-value>)",
          danger: "rgb(var(--wp-danger) / <alpha-value>)",
        },
      },
    },
  },
  plugins: [],
};
