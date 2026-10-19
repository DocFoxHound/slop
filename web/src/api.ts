// Typed client for the Slop API (crates/slop-api). Types mirror the Rust response structs.

import { isDesktop } from "./platform";

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
  platforms: AppPlatform[];
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
  is_admin: boolean;
  share_playtime: boolean;
}

export interface AdminAppRef {
  slug: string;
  title: string;
  stage: AppStage;
  status: AppStatus;
  repo_url: string;
  maker: string;
}

export interface AdminQueue {
  releases: Array<Release & { app: AdminAppRef }>;
  ready_to_list: Array<AdminAppRef & { approved_releases: number }>;
  checks: { scanning: boolean; maker_signatures_required: boolean; signing_key: boolean };
}

export interface AdminApp extends AdminAppRef {
  releases: Array<{ id: string; version: string; reviewed_at: string | null }>;
  /** Releases still being checked or waiting for review. */
  pending_releases: number;
}

export interface AuditEntry {
  id: number;
  at: string;
  actor: string;
  action: string;
  subject: string;
  detail: Record<string, unknown>;
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
  kind: "web" | "desktop";
  user_agent: string | null;
  created_at: string;
  last_seen_at: string;
  current: boolean;
}

export type Platform = "windows" | "mac" | "linux" | "android" | "linux-deb" | "linux-rpm";
/** What an app's store page says it runs on: the builds every release must include. */
export type AppPlatform = "windows" | "linux" | "linux-deb" | "linux-rpm";
export type FileKind = "archive" | "installer" | "apk";
export type ReleaseStatus = "fetching" | "awaiting_review" | "approved" | "rejected" | "failed" | "revoked";

/** Who built a file, proven by the maker's GitHub artifact attestation. */
export interface Provenance {
  source_repo: string;
  commit: string;
  signer_workflow: string;
  built_by_slop_build: boolean;
  runner: string | null;
  run_url: string | null;
}

export interface ReleaseFile {
  platform: Platform;
  kind: FileKind;
  url: string;
  file_name: string;
  exec: string | null;
  args: string[];
  size_bytes: number | null;
  sha256: string | null;
  provenance: Provenance | null;
  provenance_problem: string | null;
  scan_result: "passed" | "note" | "on_hold" | "rejected" | "blocked" | "skipped" | null;
  scan_detail: string | null;
}

export interface Release {
  id: string;
  version: string;
  commit_sha: string;
  status: ReleaseStatus;
  status_reason?: string;
  commit_verified: boolean;
  created_at: string;
  /** Who sent it: the maker, Slop Build, or Slop's GitHub release watcher. */
  source: "maker" | "slop_build" | "github_release";
  /** Approved by Slop as an update, without waiting for a person. */
  auto_approved: boolean;
  files: ReleaseFile[];
}

/** One of the repository's GitHub releases, for "Pick a release". */
export interface GithubRelease {
  tag: string;
  name: string | null;
  version: string;
  published_at: string | null;
  prerelease: boolean;
  html_url: string;
  /** Already sent to Slop. */
  submitted: boolean;
  assets: Array<{ name: string; url: string; size: number; platform: Platform | null; kind: FileKind | null }>;
}

export interface UpdateSettings {
  watch_releases: boolean;
  auto_update: boolean;
  auto_approval_ready: boolean;
  auto_approval_note: string | null;
  watch_note: string | null;
  watch_last_tag: string | null;
  watch_checked_at: string | null;
  watch_problem: string | null;
}

export interface NewRelease {
  version: string;
  commit_sha: string;
  files: Array<{ platform: Platform; kind: FileKind; url: string; exec?: string; args?: string[] }>;
}

export interface LibraryEntry extends AppSummary {
  added_at: string;
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
  platforms: AppPlatform[];
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

/** The website calls its own origin; in development Vite proxies /v1 to the API. */
export const API_BASE = "";

export function query(params: object): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== "") search.set(key, String(value));
  }
  const s = search.toString();
  return s ? `?${s}` : "";
}

/** In the desktop app, requests go through Rust, which holds the user's tokens. */
async function desktopRequest(method: string, path: string, body?: unknown): Promise<{ status: number; body: unknown }> {
  const { invoke } = await import("@tauri-apps/api/core");
  try {
    return await invoke<{ status: number; body: unknown }>("api_request", { methodName: method, path, body: body ?? null });
  } catch (err) {
    const e = err as { code?: string; message?: string };
    throw new ApiError(0, e.code ?? "network", e.message ?? "Can't reach Slop. Check your connection and try again.");
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  if (isDesktop) {
    const res = await desktopRequest(method, path, body);
    if (res.status >= 200 && res.status < 300) return res.body as T;
    const err = (res.body as { error?: { code?: string; message?: string; field?: string } } | null)?.error;
    throw new ApiError(res.status, err?.code ?? "http", err?.message ?? `Request failed (${res.status})`, err?.field);
  }

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
  /** The licence GitHub finds in a repository, for the Publish form. */
  detectLicense: (repoUrl: string) =>
    request<{ spdx: string | null; name: string | null; note: string | null }>(
      "GET",
      `/v1/repos/license${query({ url: repoUrl })}`,
    ),
  releases: (slug: string) => request<Release[]>("GET", `/v1/apps/${encodeURIComponent(slug)}/releases`),
  githubReleases: (slug: string) => request<GithubRelease[]>("GET", `/v1/apps/${encodeURIComponent(slug)}/github-releases`),
  submitGithubRelease: (slug: string, tag: string, files: NewRelease["files"]) =>
    request<Release>("POST", `/v1/apps/${encodeURIComponent(slug)}/releases/from-github`, { tag, files }),
  updateSettings: (slug: string) => request<UpdateSettings>("GET", `/v1/apps/${encodeURIComponent(slug)}/update-settings`),
  changeUpdateSettings: (slug: string, change: Partial<Pick<UpdateSettings, "watch_releases" | "auto_update">>) =>
    request<UpdateSettings>("PUT", `/v1/apps/${encodeURIComponent(slug)}/update-settings`, change),
  submitRelease: (slug: string, release: NewRelease) =>
    request<Release>("POST", `/v1/apps/${encodeURIComponent(slug)}/releases`, release),
  /** Whether an approved, signed build exists (the manifest itself is checked in Rust). */
  hasBuild: (slug: string) =>
    request<unknown>("GET", `/v1/apps/${encodeURIComponent(slug)}/manifest`).then(
      () => true,
      (err: unknown) => {
        if (err instanceof ApiError && err.status === 404) return false;
        throw err;
      },
    ),
  slopBuildSetup: (slug: string) =>
    request<{
      supported: boolean;
      project_type: "rust" | "go" | "custom";
      detected: string | null;
      note: string | null;
      file_path: string;
      workflow: string;
      create_url: string | null;
    }>("GET", `/v1/apps/${encodeURIComponent(slug)}/slop-build`),
  library: () => request<LibraryEntry[]>("GET", "/v1/me/library"),
  admin: {
    queue: () => request<AdminQueue>("GET", "/v1/admin/queue"),
    apps: (status?: AppStatus) => request<AdminApp[]>("GET", `/v1/admin/apps${query({ status })}`),
    audit: () => request<AuditEntry[]>("GET", "/v1/admin/audit"),
    approve: (id: string) => request<void>("POST", `/v1/admin/releases/${id}/approve`),
    reject: (id: string, reason: string) => request<void>("POST", `/v1/admin/releases/${id}/reject`, { reason }),
    revoke: (id: string, reason: string) => request<void>("POST", `/v1/admin/releases/${id}/revoke`, { reason }),
    list: (slug: string) => request<void>("POST", `/v1/admin/apps/${encodeURIComponent(slug)}/list`),
    delist: (slug: string, reason: string) =>
      request<void>("POST", `/v1/admin/apps/${encodeURIComponent(slug)}/delist`, { reason }),
    stage: (slug: string, stage: AppStage) =>
      request<void>("POST", `/v1/admin/apps/${encodeURIComponent(slug)}/stage`, { stage }),
  },
  addToLibrary: (slug: string) => request<void>("PUT", `/v1/me/library/${encodeURIComponent(slug)}`),
  removeFromLibrary: (slug: string) => request<void>("DELETE", `/v1/me/library/${encodeURIComponent(slug)}`),
  desktopRequest: (id: string) =>
    request<{ device_name: string; expires_at: string }>("GET", `/v1/auth/desktop/requests/${encodeURIComponent(id)}`),
  approveDesktop: (id: string) =>
    request<{ redirect_url: string }>("POST", `/v1/auth/desktop/requests/${encodeURIComponent(id)}/approve`),

  providers: () => request<{ github: boolean }>("GET", "/v1/auth/providers"),
  /** The newest desktop app build for a platform (for the download button). */
  desktopLatest: (platform: "windows" | "mac" | "linux-deb" | "linux-rpm") =>
    request<{ version: string; url: string }>("GET", `/v1/desktop/latest/${platform}`),
  /** What this server checks, so pages only claim what's true. */
  server: () => request<{ scanning: boolean; maker_signatures_required: boolean }>("GET", "/v1/server"),
  /** Full-page navigation target that starts GitHub sign-in. */
  githubSignInUrl: (returnTo: string) => `${API_BASE}/v1/auth/github/start${query({ return_to: returnTo })}`,
  signupInfo: () => request<SignupInfo>("GET", "/v1/auth/signup"),
  signup: (body: { display_name: string; country_code: string; age_confirmed: boolean }) =>
    request<{ user: Me; return_to: string }>("POST", "/v1/auth/signup", body),
  signout: () => request<void>("POST", "/v1/auth/signout"),
  me: () => request<Me>("GET", "/v1/me"),
  updateMe: (changes: { share_playtime?: boolean }) => request<void>("PATCH", "/v1/me", changes),
  players: (slug: string) =>
    request<{ playing_now: number; peak_this_month: number; daily_peaks: Array<{ day: string; peak: number }> }>(
      "GET",
      `/v1/apps/${encodeURIComponent(slug)}/players`,
    ),
  sessions: () => request<SessionInfo[]>("GET", "/v1/me/sessions"),
  revokeSession: (id: string) => request<void>("DELETE", `/v1/me/sessions/${encodeURIComponent(id)}`),
};
