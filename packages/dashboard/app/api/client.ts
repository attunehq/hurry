import type { ExchangeResponse } from "./types";

export type ApiError = {
  status: number;
  message: string;
  bodyText?: string;
};

function apiOrigin() {
  const origin = import.meta.env.VITE_API_ORIGIN as string | undefined;
  if (!origin) return "";
  return origin.replace(/\/+$/, "");
}

export function apiUrl(path: string) {
  const origin = apiOrigin();
  if (!origin) return path;
  const normalized = path.startsWith("/") ? path : `/${path}`;
  return `${origin}${normalized}`;
}

async function readBodyTextSafe(res: Response) {
  try {
    return await res.text();
  } catch {
    return "";
  }
}

export async function apiRequest<T>(args: {
  path: string;
  method?: "GET" | "POST" | "PATCH" | "DELETE";
  sessionToken?: string | null;
  body?: unknown;
  onUnauthorized?: () => void;
}): Promise<T> {
  const res = await fetch(apiUrl(args.path), {
    method: args.method ?? "GET",
    headers: {
      "content-type": "application/json",
      ...(args.sessionToken ? { authorization: `Bearer ${args.sessionToken}` } : {}),
    },
    body: args.body === undefined ? undefined : JSON.stringify(args.body),
  });

  if (!res.ok) {
    const bodyText = await readBodyTextSafe(res);

    if (res.status === 401 && args.onUnauthorized) {
      args.onUnauthorized();
    }

    throw {
      status: res.status,
      message: bodyText || res.statusText || "Request failed",
      bodyText,
    } satisfies ApiError;
  }

  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export async function exchangeAuthCode(authCode: string) {
  return await apiRequest<ExchangeResponse>({
    path: "/api/v1/oauth/exchange",
    method: "POST",
    body: { auth_code: authCode },
  });
}
