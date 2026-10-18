// Typed client for the Slop API (crates/slop-api). Types mirror the Rust response structs.

export type AppKind = "game" | "app";
export type AppStage = "coming_soon" | "beta" | "released";
export type AppStatus = "draft" | "listed" | "delisted";

export interface TagRef {
  slug: string;
  name: string;
}

export interface Category {
  slug: string;
  name: string;
  description: string;
  tags: TagRef[];
}

export interface AppSummary {
  slug: string;
  title: string;
  summary: string;
  kind: AppKind;
  stage: AppStage;
  /** Only present when not listed (the maker's own drafts). */
  status?: AppStatus;
  min_age: number;
  tags: string[];
  is_dead: boolean;
  stage_deadline: string | null;
  updated_at: string;
}

export interface AppDetail extends AppSummary {
  description_md: string;
  repo_url: string;
  license_spdx: string;
  maker: { display_name: string };
  stage_entered_at: string;
  last_commit_at: string | null;
  tag_details: TagRef[];
  is_mine: boolean;
}

export interface AppPage {
  apps: AppSummary[];
  next_offset: number | null;
}

export interface Me {
  id: string;
  display_name: string;
  email: string | null;
  country_code: string;
}

export interface Country {
  code: string;
  name: string;
  min_age: number;
}

export interface SignupInfo {
  suggested_name: string;
  email: string | null;
  countries: Country[];
}

export interface SessionInfo {
  id: string;
  user_agent: string | null;
  created_at: string;
  last_seen_at: string;
  current: boolean;
}

export interface NewApp {
  slug: string;
  title: string;
  summary: string;
  description_md: string;
  kind: AppKind;
  stage: AppStage;
  repo_url: string;
  license_spdx: string;
  min_age: number;
  tags: string[];
}

export type AppChanges = Partial<Omit<NewApp, "slug" | "kind" | "stage">>;

export interface AppQuery {
  q?: string;
  tag?: string;
  category?: string;
  stage?: AppStage;
  kind?: AppKind;
  sort?: "recent" | "title" | "relevance";
  limit?: number;
  offset?: number;
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly field?: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/**
 * Where the API lives. The website and the desktop app in development use the same origin
 * (Vite proxies /v1). A packaged desktop app sets VITE_API_BASE at build time.
 */
export const API_BASE: string = (import.meta.env.VITE_API_BASE as string | undefined) ?? "";

export function query(params: object): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== "") search.set(key, String(value));
  }
  const s = search.toString();
  return s ? `?${s}` : "";
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const init: RequestInit = {
    method,
    headers: { Accept: "application/json" },
    // Cookies only travel same-origin; the packaged desktop app reads the API without them.
    credentials: "same-origin",
  };
  if (body !== undefined) {
    init.headers = { ...init.headers, "Content-Type": "application/json" };
    init.body = JSON.stringify(body);
  }

  let res: Response;
  try {
    res = await fetch(API_BASE + path, init);
  } catch {
    throw new ApiError(0, "network", "Can't reach Slop. Check your connection and try again.");
  }
  if (res.status === 204) return undefined as T;

  const data: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    const err = (data as { error?: { code?: string; message?: string; field?: string } } | null)?.error;
    throw new ApiError(res.status, err?.code ?? "http", err?.message ?? `Request failed (${res.status})`, err?.field);
  }
  return data as T;
}

export const api = {
  categories: () => request<Category[]>("GET", "/v1/categories"),
  apps: (q: AppQuery = {}) => request<AppPage>("GET", `/v1/apps${query(q)}`),
  app: (slug: string) => request<AppDetail>("GET", `/v1/apps/${encodeURIComponent(slug)}`),
  createApp: (app: NewApp) => request<AppDetail>("POST", "/v1/apps", app),
  updateApp: (slug: string, changes: AppChanges) =>
    request<AppDetail>("PATCH", `/v1/apps/${encodeURIComponent(slug)}`, changes),
  myApps: () => request<AppSummary[]>("GET", "/v1/me/apps"),

  providers: () => request<{ github: boolean }>("GET", "/v1/auth/providers"),
  /** Full-page navigation target that starts GitHub sign-in. */
  githubSignInUrl: (returnTo: string) => `${API_BASE}/v1/auth/github/start${query({ return_to: returnTo })}`,
  signupInfo: () => request<SignupInfo>("GET", "/v1/auth/signup"),
  signup: (body: { display_name: string; country_code: string; age_confirmed: boolean }) =>
    request<{ user: Me; return_to: string }>("POST", "/v1/auth/signup", body),
  signout: () => request<void>("POST", "/v1/auth/signout"),
  me: () => request<Me>("GET", "/v1/me"),
  sessions: () => request<SessionInfo[]>("GET", "/v1/me/sessions"),
  revokeSession: (id: string) => request<void>("DELETE", `/v1/me/sessions/${encodeURIComponent(id)}`),
};
