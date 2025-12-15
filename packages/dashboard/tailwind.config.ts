import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: {
          950: "#070A12",
          900: "#0B1020",
          800: "#0F1631",
          700: "#141E42",
        },
        neon: {
          500: "#7C5CFF",
          400: "#8B7BFF",
          300: "#B6ADFF",
        },
        aurora: {
          500: "#22D3EE",
          400: "#34D399",
          300: "#A3E635",
        },
      },
      boxShadow: {
        glow: "0 0 0 1px rgba(124,92,255,.35), 0 10px 40px rgba(124,92,255,.18)",
        "glow-soft":
          "0 0 0 1px rgba(124,92,255,.25), 0 8px 30px rgba(124,92,255,.12)",
      },
      backgroundImage: {
        "grid-fade":
          "radial-gradient(ellipse at top, rgba(124,92,255,.15), transparent 55%), radial-gradient(ellipse at bottom, rgba(34,211,238,.10), transparent 55%)",
      },
    },
  },
  plugins: [],
} satisfies Config;

