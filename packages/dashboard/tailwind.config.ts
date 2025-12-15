import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: {
          950: "#070A0C",
          900: "#0A100F",
          800: "#0E1614",
          700: "#131E1A",
        },
        // Attune brand teal/mint gradient colors
        attune: {
          600: "#2D9B7B",
          500: "#5AC7A3",
          400: "#6AD4B0",
          300: "#8FE0C4",
        },
        // Keep neon for accent compatibility, but shift to teal
        neon: {
          500: "#3DA88A",
          400: "#5AC7A3",
          300: "#8FE0C4",
        },
        aurora: {
          500: "#22D3EE",
          400: "#5AC7A3",
          300: "#8FE0C4",
        },
      },
      boxShadow: {
        glow: "0 0 0 1px rgba(90,199,163,.35), 0 10px 40px rgba(90,199,163,.18)",
        "glow-soft":
          "0 0 0 1px rgba(90,199,163,.25), 0 8px 30px rgba(90,199,163,.12)",
      },
      backgroundImage: {
        "grid-fade":
          "radial-gradient(ellipse at top, rgba(90,199,163,.15), transparent 55%), radial-gradient(ellipse at bottom, rgba(34,211,238,.10), transparent 55%)",
      },
    },
  },
  plugins: [],
} satisfies Config;

