import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // Semantic surface colors (theme-aware)
        surface: {
          base: "var(--surface-base)",
          raised: "var(--surface-raised)",
          overlay: "var(--surface-overlay)",
          subtle: "var(--surface-subtle)",
          "subtle-hover": "var(--surface-subtle-hover)",
        },
        // Semantic border colors
        border: {
          DEFAULT: "var(--border-default)",
          subtle: "var(--border-subtle)",
          accent: "var(--border-accent)",
          "accent-hover": "var(--border-accent-hover)",
        },
        // Semantic text colors
        content: {
          primary: "var(--text-primary)",
          secondary: "var(--text-secondary)",
          tertiary: "var(--text-tertiary)",
          muted: "var(--text-muted)",
        },
        // Accent colors (theme-aware)
        accent: {
          bold: "var(--accent-bold)",
          muted: "var(--accent-muted)",
          subtle: "var(--accent-subtle)",
          text: "var(--accent-text)",
        },
        // Danger colors (theme-aware)
        danger: {
          bg: "var(--danger-bg)",
          "bg-hover": "var(--danger-bg-hover)",
          border: "var(--danger-border)",
          text: "var(--danger-text)",
        },
        // Keep attune for gradient text (static brand colors)
        attune: {
          600: "#2D9B7B",
          500: "#5AC7A3",
          400: "#6AD4B0",
          300: "#8FE0C4",
        },
      },
      borderColor: {
        DEFAULT: "var(--border-default)",
      },
      boxShadow: {
        glow: "var(--glow)",
        "glow-soft": "var(--glow-soft)",
      },
      backgroundColor: {
        backdrop: "var(--backdrop-overlay)",
      },
    },
  },
  plugins: [],
} satisfies Config;
