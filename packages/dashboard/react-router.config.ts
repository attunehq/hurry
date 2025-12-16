import type { Config } from "@react-router/dev/config";

export default {
  ssr: false, // Pure SPA mode - produces static files
  appDirectory: "app",
} satisfies Config;
