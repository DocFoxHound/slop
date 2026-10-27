// SPDX-License-Identifier: AGPL-3.0-or-later
// Handoff links (DESIGN §8 and §15.1): someone's AI assistant prepares a new app's store page or a
// People page and gives them one link, /publish/new#fill=… or /people/me#fill=…, with the fields
// as URL-encoded JSON (`encodeURIComponent(JSON.stringify(fields))`, named as the API names them).
// The fill rides in the fragment, so it never reaches the server or its logs. The form opens
// filled in; the person checks it and presses Create themselves.
//
// The link is untrusted: only the form's own fields, of the right type and within the form's
// limits, go into the form, and only as the values of its inputs. Nothing that's the person's own
// to decide is ever filled in: being 18 or older, agreements, whether they work as a business,
// payments, or a token.

import {
  ACCESSIBILITY,
  type Accent,
  type AiUse,
  type AppPlatform,
  type ContentLabel,
  type ModsFolder,
  type NewApp,
  type OnlineKind,
  type ProfileInput,
  type Section,
  type Service,
} from "./api";
import { CONTENT_LABEL, ONLINE_LABEL } from "./format";

export type HandoffForm = "app" | "people";

/** A link's fields as sent, or why the link couldn't be read. */
export type HandoffLink = { fields: Record<string, unknown> } | { error: string };

/** What a link fills in on a form, and what it couldn't. */
export interface Fill<T> {
  values: Partial<T>;
  /** The fields filled in, by name, in the form's order. */
  filled: Array<keyof T & string>;
  /** Fields left out (or partly), by name, and why. */
  left: Array<{ field: string; why: string }>;
  /** How many of the link's fields this form doesn't have. */
  unknown: number;
}

/** A People page's fill, plus what's only ever a suggestion. */
export interface PeopleFill extends Fill<PeopleFields> {
  /** Whether the assistant thinks they offer services as a business: shown, never chosen for them. */
  traderSuggested?: boolean;
}

/** The People editor's fields, with the portfolio repository from its Portfolio panel. */
export type PeopleFields = Omit<ProfileInput, "trader" | "confirm_adult"> & { portfolio_repo: string };

/** The longest link read, encoded (a full description in any script fits well within it). */
const MAX_LINK = 500_000;

/** The fields in a `#fill=` fragment; null when the address has no fill. */
export function decodeFill(hash: string): HandoffLink | null {
  if (!hash.startsWith("#fill=")) return null;
  const raw = hash.slice("#fill=".length);
  if (raw.length > MAX_LINK) return { error: "it's too long" };
  let text: string;
  try {
    text = decodeURIComponent(raw);
  } catch {
    return { error: "it isn't URL-encoded properly" };
  }
  return fieldsOf(text);
}

function fieldsOf(text: string): HandoffLink {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return { error: "what follows #fill= isn't JSON" };
  }
  if (!isRecord(value)) return { error: 'it should hold one JSON object of fields, like {"title": "…"}' };
  return { fields: value };
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** The object's own key (never one it inherits, like `toString`). */
const has = (o: object, key: string) => Object.prototype.hasOwnProperty.call(o, key);

// ---- Kept in the tab -----------------------------------------------------------------------------
//
// Signing in (and making an account) goes through the sign-in providers' pages and back, which
// drops the fragment: the fields are kept in this tab's sessionStorage until the form uses them.

/** Long enough to make an account; a fill found later isn't applied. */
export const KEEP_FOR_MS = 2 * 60 * 60 * 1000;

const keyOf = (form: HandoffForm | "portfolio") => `slop.handoff.${form}`;

function store(key: string, value: string | null) {
  try {
    if (value === null) sessionStorage.removeItem(key);
    else sessionStorage.setItem(key, value);
  } catch {
    // Storage blocked (some private windows): the fill only lasts for this page view.
  }
}

function load(key: string): string | null {
  try {
    return sessionStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * The link's fields for this form: from the address bar, or kept from before signing in. A fill in
 * the address is kept in the tab (for the sign-in pages in between) and taken out of the address
 * bar, so a reload doesn't apply it again over what the person changed. Safe to call twice.
 */
export function takeHandoff(form: HandoffForm, now = Date.now()): HandoffLink | null {
  const link = decodeFill(window.location.hash);
  if (link) {
    window.history.replaceState(window.history.state, "", window.location.pathname + window.location.search);
    store(keyOf(form), "fields" in link ? JSON.stringify({ at: now, fields: link.fields }) : null);
    return link;
  }
  const kept = load(keyOf(form));
  if (!kept) return null;
  try {
    const { at, fields } = JSON.parse(kept) as { at?: unknown; fields?: unknown };
    if (typeof at === "number" && now - at >= 0 && now - at < KEEP_FOR_MS && isRecord(fields)) return { fields };
  } catch {
    // Not ours, or damaged: dropped below.
  }
  store(keyOf(form), null);
  return null;
}

/** The form has the fill now: it isn't applied again. */
export function forgetHandoff(form: HandoffForm) {
  store(keyOf(form), null);
}

/** A portfolio repository for a People page that doesn't exist yet: filled in once it does. */
export function keepPortfolioRepo(url: string, now = Date.now()) {
  store(keyOf("portfolio"), JSON.stringify({ at: now, url }));
}

/** The portfolio repository kept for later, if any. */
export function keptPortfolioRepo(now = Date.now()): string | undefined {
  const kept = load(keyOf("portfolio"));
  if (!kept) return undefined;
  try {
    const { at, url } = JSON.parse(kept) as { at?: unknown; url?: unknown };
    if (typeof at === "number" && now - at >= 0 && now - at < KEEP_FOR_MS) {
      const ok = httpsUrl(300, true)(url);
      if (typeof ok === "string") return ok;
    }
  } catch {
    // Damaged: dropped below.
  }
  forgetPortfolioRepo();
  return undefined;
}

/** The Portfolio panel has it now. */
export function forgetPortfolioRepo() {
  store(keyOf("portfolio"), null);
}

// ---- Checks --------------------------------------------------------------------------------------

/** Why a value can't go into the form. */
class Skip {
  constructor(readonly why: string) {}
}
/** A value that goes in, with a note on what was left out of it. */
class Part<V> {
  constructor(
    readonly value: V,
    readonly why: string,
  ) {}
}
type Check<V> = (value: unknown) => V | Skip | Part<V>;

const count = (n: number) => n.toLocaleString("en");

/** Controls other than a line break or tab in text, or any at all in a one-line field. */
function hasControl(v: string, lines: boolean): boolean {
  return lines ? /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(v) : /[\u0000-\u001f\u007f]/.test(v);
}

function text(max: number, opts: { lines?: boolean; required?: boolean } = {}): Check<string> {
  return (v) => {
    if (typeof v !== "string") return new Skip("should be text");
    // The form's own limit (maxLength counts the same way).
    if (v.length > max) return new Skip(`is over ${count(max)} characters`);
    if (opts.required && !v.trim()) return new Skip("is empty");
    if (hasControl(v, opts.lines ?? false)) return new Skip(opts.lines ? "has characters that aren't allowed" : "should be one line");
    return v;
  };
}

function pattern(re: RegExp, min: number, max: number, rule: string): Check<string> {
  return (v) => (typeof v === "string" && v.length >= min && v.length <= max && re.test(v) ? v : new Skip(rule));
}

function oneOf<T extends string | number>(choices: readonly T[]): Check<T> {
  return (v) => (choices.includes(v as T) ? (v as T) : new Skip(`should be one of ${choices.join(", ")}`));
}

const flag: Check<boolean> = (v) => (typeof v === "boolean" ? v : new Skip("should be true or false"));

function whole(min: number, max: number): Check<number> {
  return (v) =>
    typeof v === "number" && Number.isInteger(v) && v >= min && v <= max ? v : new Skip(`should be a whole number from ${count(min)} to ${count(max)}`);
}

/** An https address, as the API takes them (no user name or password in it). */
function httpsUrl(max: number, repo = false): Check<string> {
  return (v) => {
    const rule = repo ? "should be the repository's https:// address" : "should be an https:// address";
    if (typeof v !== "string" || v.length > max) return new Skip(`${rule} of up to ${max} characters`);
    let url: URL;
    try {
      url = new URL(v.trim());
    } catch {
      return new Skip(rule);
    }
    if (url.protocol !== "https:" || !url.hostname || url.username || url.password) return new Skip(rule);
    if (repo && (url.search || url.hash || url.pathname.replace(/\//g, "") === "")) return new Skip(rule);
    return v.trim();
  };
}

/**
 * A list of choices, each once. Choices it doesn't know are left out and named (`partial`), or
 * keep the whole list out.
 */
function list<T extends string>(known: (v: string) => boolean, max: number, partial = false): Check<T[]> {
  return (v) => {
    if (!Array.isArray(v)) return new Skip("should be a list");
    const out: T[] = [];
    const unknown: string[] = [];
    for (const x of v) {
      if (typeof x === "string" && known(x)) {
        if (!out.includes(x as T)) out.push(x as T);
      } else unknown.push(typeof x === "string" ? x.slice(0, 40) : JSON.stringify(x)?.slice(0, 40) ?? "?");
    }
    if (out.length > max) return new Skip(`has more than ${max}`);
    if (unknown.length === 0) return out;
    const why = `${unknown.slice(0, 5).join(", ")}${unknown.length > 5 ? "…" : ""} ${unknown.length === 1 ? "isn't one" : "aren't"} of the choices`;
    return partial ? new Part(out, why) : new Skip(why);
  };
}

/** A list of objects: the good ones go in, each bad one is named with why. */
function items<V>(max: number, what: string, check: (v: unknown) => V | Skip): Check<V[]> {
  return (v) => {
    if (!Array.isArray(v)) return new Skip("should be a list");
    if (v.length > max) return new Skip(`has more than ${max}`);
    const out: V[] = [];
    const bad: string[] = [];
    v.forEach((x, i) => {
      const r = check(x);
      if (r instanceof Skip) bad.push(`${what} ${i + 1}: ${r.why}`);
      else out.push(r);
    });
    return bad.length ? new Part(out, bad.join("; ")) : out;
  };
}

/** One field of an object, or Skip naming it. */
function field<V>(o: Record<string, unknown>, name: string, check: Check<V>, fallback?: V): V | Skip {
  if (!has(o, name) || o[name] === undefined) return fallback !== undefined ? fallback : new Skip(`${name} is missing`);
  const r = check(o[name]);
  if (r instanceof Skip) return new Skip(`${name} ${r.why}`);
  return r instanceof Part ? r.value : r;
}

type Checks<T> = { [K in keyof T]?: Check<T[K]> };

/** The fields the form knows, checked, in the form's order; `never` names what's the person's own. */
function read<T>(fields: Record<string, unknown>, checks: Checks<T>, never: Record<string, string>): Fill<T> {
  const out: Fill<T> = { values: {}, filled: [], left: [], unknown: 0 };
  for (const name of Object.keys(checks) as Array<keyof T & string>) {
    if (!has(fields, name)) continue;
    const r = checks[name]!(fields[name]);
    if (r instanceof Skip) {
      out.left.push({ field: name, why: r.why });
      continue;
    }
    if (r instanceof Part) out.left.push({ field: name, why: r.why });
    out.values[name] = (r instanceof Part ? r.value : r) as T[typeof name];
    out.filled.push(name);
  }
  for (const name of Object.keys(fields)) {
    if (has(checks, name)) continue;
    if (has(never, name)) out.left.push({ field: name, why: never[name] });
    else out.unknown++;
  }
  return out;
}

/** What's never filled in on either form, and why. */
const AGREEMENTS: Record<string, string> = {
  accept_terms: "accepting an agreement is yours to do",
  accept_agreement: "accepting an agreement is yours to do",
  terms: "accepting an agreement is yours to do",
  agreement: "accepting an agreement is yours to do",
};

// ---- New app (Publish → New app) -----------------------------------------------------------------

export const APP_LIMITS = { title: 80, slug: 60, summary: 200, description_md: 20_000, license_spdx: 64, tags: 10, online_hosts: 10, mods_path: 200, repo_url: 300 } as const;

const PLATFORMS: AppPlatform[] = ["windows", "linux", "linux-deb", "linux-rpm"];
const AGES = [0, 3, 7, 12, 16, 18];
const ACCESSIBILITY_KEYS = new Set(ACCESSIBILITY.map(([k]) => k));

const modsFolder: Check<ModsFolder | null> = (v) => {
  if (v === null) return null;
  if (!isRecord(v)) return new Skip('should be null or {"base": "game" or "saves", "path": "mods", "layout": "folder" or "flat"}');
  const base = field(v, "base", oneOf(["game", "saves"] as const));
  const path = field(v, "path", text(APP_LIMITS.mods_path, { required: true }));
  const layout = field(v, "layout", oneOf(["folder", "flat"] as const), "folder");
  for (const r of [base, path, layout]) if (r instanceof Skip) return r;
  return { base: base as ModsFolder["base"], path: path as string, layout: layout as ModsFolder["layout"] };
};

const host = /^[A-Za-z0-9.-]{1,253}(:\d{1,5})?$/;

/**
 * A new app's fields (the API's `NewApp`), checked against the form's limits. `tags`: the tags
 * the store has, when known; others are left out.
 */
export function appFill(fields: Record<string, unknown>, tags?: ReadonlySet<string>): Fill<NewApp> {
  const slug = /^[a-z0-9]+(-[a-z0-9]+)*$/;
  return read<NewApp>(
    fields,
    {
      title: text(APP_LIMITS.title, { required: true }),
      slug: pattern(slug, 3, APP_LIMITS.slug, "should be 3 to 60 lowercase letters, numbers and single dashes"),
      summary: text(APP_LIMITS.summary, { required: true }),
      description_md: text(APP_LIMITS.description_md, { lines: true }),
      visibility: oneOf(["public", "private"] as const),
      kind: oneOf(["game", "app"] as const),
      stage: oneOf(["coming_soon", "beta", "released"] as const),
      repo_url: httpsUrl(APP_LIMITS.repo_url, true),
      license_spdx: pattern(/^[A-Za-z0-9 .+()-]+$/, 1, APP_LIMITS.license_spdx, "should be an SPDX licence ID, such as MIT or GPL-3.0-only"),
      min_age: oneOf(AGES),
      content_labels: list<ContentLabel>((x) => has(CONTENT_LABEL, x), 6),
      online: list<OnlineKind>((x) => has(ONLINE_LABEL, x), 4),
      online_hosts: list<string>((x) => host.test(x), APP_LIMITS.online_hosts),
      sandbox_ready: flag,
      bug_reports: flag,
      accessibility: list<string>((x) => ACCESSIBILITY_KEYS.has(x), ACCESSIBILITY.length),
      mods_folder: modsFolder,
      platforms: list<AppPlatform>((x) => (PLATFORMS as string[]).includes(x), PLATFORMS.length),
      tags: list<string>((x) => (tags ? tags.has(x) : slug.test(x)), APP_LIMITS.tags, true),
    },
    {
      ...AGREEMENTS,
      repo_token: "a token never goes in a link: if the repository is private, paste a read-only token yourself",
    },
  );
}

/** The new app form's labels, for saying what was filled in. */
export const APP_LABELS: Record<string, string> = {
  title: "Name",
  slug: "Store address",
  summary: "One-line summary",
  description_md: "Description",
  visibility: "Who can see it",
  kind: "Type",
  stage: "Stage",
  repo_url: "Source repository",
  license_spdx: "Licence",
  min_age: "Age rating",
  content_labels: "Content labels",
  online: "Online",
  online_hosts: "Your servers",
  sandbox_ready: "Works in sandbox mode",
  bug_reports: "Take bug reports",
  accessibility: "Accessibility",
  mods_folder: "Mods",
  platforms: "Runs on",
  tags: "Tags",
  repo_token: "Repository token",
};

// ---- People page (/people/me) --------------------------------------------------------------------

export const PEOPLE_LIMITS = {
  name: 60,
  handle: 40,
  headline: 140,
  location: 60,
  turnaround: 40,
  ai_note: 300,
  about_md: 6000,
  services: 8,
  service_name: 60,
  service_description: 600,
  links: 5,
  link_label: 30,
  url: 300,
} as const;

/** Prices in minor units (cents, pence), as the API takes them (PAYMENTS §7). */
export const PRICE_MINOR = { min: 1_000, max: 1_000_000 } as const;

const ACCENT_NAMES: Accent[] = ["slime", "ocean", "grape", "rose", "ember", "sun", "forest", "slate"];
const SECTION_NAMES: Section[] = ["portfolio", "services", "reviews", "worked_on", "about"];
const skillSlug = /^[a-z0-9]+(-[a-z0-9]+)*$/;

function service(skills?: ReadonlySet<string>): (v: unknown) => Service | Skip {
  return (v) => {
    if (!isRecord(v)) return new Skip("should be an object");
    const name = field(v, "name", text(PEOPLE_LIMITS.service_name, { required: true }));
    const description = field(v, "description", text(PEOPLE_LIMITS.service_description), "");
    const price = field(v, "price_minor", whole(PRICE_MINOR.min, PRICE_MINOR.max));
    const currency = field(v, "currency", oneOf(["usd", "eur", "gbp"] as const));
    const days = field(v, "days", whole(1, 365));
    const skill = field(v, "skill", (x) => (typeof x === "string" && (skills ? skills.has(x) : skillSlug.test(x)) ? x : new Skip("isn't a skill")));
    const id = field(v, "id", pattern(/^[0-9a-f]{8}$/, 8, 8, "isn't a service's id"), "");
    for (const r of [name, description, price, currency, days, skill, id]) if (r instanceof Skip) return r;
    return {
      id: id as string,
      name: name as string,
      description: description as string,
      price_minor: price as number,
      currency: currency as Service["currency"],
      days: days as number,
      skill: skill as string,
    };
  };
}

const link = (v: unknown): { label: string; url: string } | Skip => {
  if (!isRecord(v)) return new Skip("should be an object");
  const label = field(v, "label", text(PEOPLE_LIMITS.link_label), "");
  const url = field(v, "url", httpsUrl(PEOPLE_LIMITS.url));
  for (const r of [label, url]) if (r instanceof Skip) return r;
  return { label: label as string, url: url as string };
};

/**
 * A People page's fields (the API's `ProfileInput`, and `portfolio_repo`), checked against the
 * editor's limits. `skills`: the skills People has, when known. `trader` is only ever a
 * suggestion; being 18 or older and agreements are never filled in.
 */
export function peopleFill(fields: Record<string, unknown>, skills?: ReadonlySet<string>): PeopleFill {
  const knownSkill = (x: string) => (skills ? skills.has(x) : skillSlug.test(x));
  // Business or not is the person's legal status: a suggestion, never an answer.
  const { trader, ...rest } = fields;
  const out: PeopleFill = read<PeopleFields>(
    rest,
    {
      name: text(PEOPLE_LIMITS.name, { required: true }),
      handle: pattern(/^[a-z0-9]+(-[a-z0-9]+)*$/, 3, PEOPLE_LIMITS.handle, "should be 3 to 40 lowercase letters, numbers and single dashes"),
      kind: oneOf(["individual", "studio"] as const),
      team_size: (v) => (v === null ? null : whole(2, 500)(v)),
      headline: text(PEOPLE_LIMITS.headline),
      skills: list<string>(knownSkill, 20, true),
      ai_use: oneOf<AiUse>(["none", "assisted", "heavy"]),
      ai_note: text(PEOPLE_LIMITS.ai_note),
      location: text(PEOPLE_LIMITS.location),
      turnaround: text(PEOPLE_LIMITS.turnaround),
      taking_work: flag,
      about_md: text(PEOPLE_LIMITS.about_md, { lines: true }),
      services: items(PEOPLE_LIMITS.services, "service", service(skills)),
      accent: oneOf(ACCENT_NAMES),
      sections: (v) => {
        const r = list<Section>((x) => (SECTION_NAMES as string[]).includes(x), SECTION_NAMES.length)(v);
        // Every section once, in the order given; any left out go at the end (as the API does).
        return Array.isArray(r) ? [...r, ...SECTION_NAMES.filter((s) => !r.includes(s))] : r;
      },
      links: items(PEOPLE_LIMITS.links, "link", link),
      portfolio_repo: httpsUrl(PEOPLE_LIMITS.url, true),
    },
    {
      ...AGREEMENTS,
      confirm_adult: "only you can say you're 18 or older",
      adult: "only you can say you're 18 or older",
      stripe: "payments are set up by you, with Stripe",
      payments: "payments are set up by you, with Stripe",
    },
  );
  if (typeof trader === "boolean") out.traderSuggested = trader;
  // A service's skill is one of theirs: the editor offers services only for the skills picked.
  const v = out.values;
  const used = (v.services ?? []).map((s) => s.skill).filter((s) => !(v.skills ?? []).includes(s));
  if (used.length) {
    v.skills = [...(v.skills ?? []), ...used.filter((s, i) => used.indexOf(s) === i)];
    if (!out.filled.includes("skills")) out.filled.splice(out.filled.indexOf("services"), 0, "skills");
  }
  if (v.kind === "individual") v.team_size = null;
  return out;
}

/** The People editor's labels, for saying what was filled in. */
export const PEOPLE_LABELS: Record<string, string> = {
  name: "Name",
  handle: "Page address",
  kind: "Who",
  team_size: "People in the studio",
  headline: "Headline",
  skills: "What you do",
  ai_use: "How you use AI",
  ai_note: "AI use note",
  location: "Location",
  turnaround: "Usual turnaround",
  taking_work: "Taking work now",
  about_md: "About",
  services: "Services",
  accent: "Accent colour",
  sections: "Section order",
  links: "Links",
  portfolio_repo: "Portfolio repository",
  confirm_adult: "I'm 18 or older",
  adult: "I'm 18 or older",
  stripe: "Get paid",
  payments: "Get paid",
};
