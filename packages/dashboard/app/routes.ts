import { index, route, type RouteConfig } from "@react-router/dev/routes";

export default [
  index("routes/_index.tsx"),
  route("auth", "routes/auth.tsx"),
  route("auth/callback", "routes/auth.callback.tsx"),
  route("billing", "routes/billing.tsx"),
  route("user", "routes/user.tsx"),
  route("invite/:token", "routes/invite.$token.tsx"),
  route("org/:orgId", "routes/org.$orgId.tsx", [
    index("routes/org.$orgId/_index.tsx"),
    route("members", "routes/org.$orgId/members.tsx"),
    route("api-keys", "routes/org.$orgId/api-keys.tsx"),
    route("invitations", "routes/org.$orgId/invitations.tsx"),
    route("bots", "routes/org.$orgId/bots.tsx"),
  ]),
] satisfies RouteConfig;
