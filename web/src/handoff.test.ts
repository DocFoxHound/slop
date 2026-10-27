// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  appFill,
  decodeFill,
  forgetHandoff,
  forgetPortfolioRepo,
  keepPortfolioRepo,
  keptPortfolioRepo,
  KEEP_FOR_MS,
  peopleFill,
  takeHandoff,
} from "./handoff";

const link = (fields: unknown) => `#fill=${encodeURIComponent(JSON.stringify(fields))}`;

const APP = {
  title: "Night Ferry",
  slug: "night-ferry",
  summary: "A ferry at night.",
  description_md: "# Night Ferry\n\nCross the bay.",
  visibility: "public",
  kind: "game",
  stage: "beta",
  repo_url: "https://github.com/you/night-ferry",
  license_spdx: "MIT",
  min_age: 7,
  content_labels: ["horror"],
  online: ["own_servers"],
  online_hosts: ["play.example.com:7777"],
  sandbox_ready: true,
  bug_reports: false,
  accessibility: ["subtitles"],
  mods_folder: { base: "game", path: "mods", layout: "folder" },
  platforms: ["windows", "linux", "linux-deb"],
  tags: ["puzzle", "relaxing"],
};

const PERSON = {
  name: "Sea Sounds",
  handle: "sea-sounds",
  kind: "studio",
  team_size: 3,
  headline: "Waves, gulls and foghorns",
  skills: ["sound", "music"],
  ai_use: "assisted",
  ai_note: "Mood boards only.",
  location: "Lisbon, PT",
  turnaround: "1 week",
  taking_work: true,
  about_md: "We record **real** foley.",
  services: [{ name: "Harbour pack", description: "40 sounds, WAV.", price_minor: 3500, currency: "eur", days: 5, skill: "sound" }],
  accent: "ocean",
  sections: ["services", "portfolio"],
  links: [{ label: "Showreel", url: "https://soundcloud.com/sea" }],
  portfolio_repo: "https://github.com/sea/portfolio",
};

const TAGS = new Set(["puzzle", "relaxing", "horror"]);
const SKILLS = new Set(["sound", "music", "pixel-art"]);

describe("reading a #fill= link", () => {
  it("is only for #fill= fragments", () => {
    expect(decodeFill("")).toBeNull();
    expect(decodeFill("#donations")).toBeNull();
    expect(decodeFill("#portfolio")).toBeNull();
  });

  it("reads URL-encoded JSON", () => {
    expect(decodeFill(link({ title: "Ä ferry ⛴" }))).toEqual({ fields: { title: "Ä ferry ⛴" } });
    // Browsers encode spaces and quotes typed straight into the address bar; that reads too.
    expect(decodeFill('#fill={%22title%22:%20%22Ferry%22}')).toEqual({ fields: { title: "Ferry" } });
  });

  it("says why a link can't be read", () => {
    expect(decodeFill("#fill=%E0%A4%A")).toEqual({ error: "it isn't URL-encoded properly" });
    expect(decodeFill("#fill=")).toEqual({ error: "what follows #fill= isn't JSON" });
    expect(decodeFill("#fill=%7Btitle")).toEqual({ error: "what follows #fill= isn't JSON" });
    for (const v of [[1, 2], "text", 3, null]) {
      expect(decodeFill(link(v))).toEqual({ error: 'it should hold one JSON object of fields, like {"title": "…"}' });
    }
    expect(decodeFill(`#fill=${"a".repeat(500_001)}`)).toEqual({ error: "it's too long" });
  });
});

describe("a new app's fill", () => {
  it("fills in every field the form has, in the form's order", () => {
    const fill = appFill(APP, TAGS);
    expect(fill.values).toEqual(APP);
    expect(fill.filled).toEqual(Object.keys(APP));
    expect(fill.left).toEqual([]);
    expect(fill.unknown).toBe(0);
  });

  it("ignores fields it doesn't have, and never takes a token or an agreement", () => {
    const fill = appFill({ title: "Ferry", price: 5, icon: "x.png", repo_token: { kind: "gitea", token: "secret" }, accept_terms: true }, TAGS);
    expect(fill.values).toEqual({ title: "Ferry" });
    expect(fill.unknown).toBe(2);
    expect(fill.left.map((l) => l.field)).toEqual(["repo_token", "accept_terms"]);
    expect(JSON.stringify(fill)).not.toContain("secret");
  });

  it("holds every field to the form's limits and types", () => {
    const fill = appFill(
      {
        title: "x".repeat(81),
        slug: "Night Ferry",
        summary: "two\nlines",
        description_md: "y".repeat(20_001),
        visibility: "friends",
        kind: "film",
        stage: 2,
        repo_url: "http://github.com/you/ferry",
        license_spdx: "MIT; rm -rf",
        min_age: 13,
        content_labels: ["horror", "toString"],
        online: "own_servers",
        online_hosts: ["not a host!!"],
        sandbox_ready: "yes",
        bug_reports: 1,
        accessibility: ["subtitles", "__proto__"],
        mods_folder: { base: "elsewhere", path: "mods" },
        platforms: ["windows", "mac"],
      },
      TAGS,
    );
    expect(fill.values).toEqual({});
    expect(fill.filled).toEqual([]);
    expect(Object.fromEntries(fill.left.map((l) => [l.field, l.why]))).toEqual({
      title: "is over 80 characters",
      slug: "should be 3 to 60 lowercase letters, numbers and single dashes",
      summary: "should be one line",
      description_md: "is over 20,000 characters",
      visibility: "should be one of public, private",
      kind: "should be one of game, app",
      stage: "should be one of coming_soon, beta, released",
      repo_url: "should be the repository's https:// address",
      license_spdx: "should be an SPDX licence ID, such as MIT or GPL-3.0-only",
      min_age: "should be one of 0, 3, 7, 12, 16, 18",
      content_labels: "toString isn't one of the choices",
      online: "should be a list",
      online_hosts: "not a host!! isn't one of the choices",
      sandbox_ready: "should be true or false",
      bug_reports: "should be true or false",
      accessibility: "__proto__ isn't one of the choices",
      mods_folder: "base should be one of game, saves",
      platforms: "mac isn't one of the choices",
    });
  });

  it("takes a repository's https address only, as the API does", () => {
    const repo = (repo_url: string) => appFill({ repo_url }).values.repo_url;
    expect(repo(" https://gitlab.com/you/ferry ")).toBe("https://gitlab.com/you/ferry");
    expect(repo("https://github.com/you/ferry?tab=readme")).toBeUndefined();
    expect(repo("https://github.com/you/ferry#readme")).toBeUndefined();
    expect(repo("https://me:pw@github.com/you/ferry")).toBeUndefined();
    expect(repo("https://github.com/")).toBeUndefined();
    expect(repo("javascript:alert(1)")).toBeUndefined();
    expect(repo(`https://github.com/you/${"f".repeat(300)}`)).toBeUndefined();
  });

  it("keeps the store's tags and names the ones it doesn't have", () => {
    const fill = appFill({ tags: ["puzzle", "made-up", "puzzle", "relaxing"] }, TAGS);
    expect(fill.values.tags).toEqual(["puzzle", "relaxing"]);
    expect(fill.filled).toEqual(["tags"]);
    expect(fill.left).toEqual([{ field: "tags", why: "made-up isn't one of the choices" }]);
    // Ten at most, like the form.
    const many = Array.from({ length: 11 }, (_, i) => `tag-${i}`);
    expect(appFill({ tags: many }).left).toEqual([{ field: "tags", why: "has more than 10" }]);
  });

  it("a mods folder: null for none, a layout of each mod in its own folder unless said", () => {
    expect(appFill({ mods_folder: null }).values).toEqual({ mods_folder: null });
    expect(appFill({ mods_folder: { base: "saves", path: "Content/Paks/~mods" } }).values.mods_folder).toEqual({
      base: "saves",
      path: "Content/Paks/~mods",
      layout: "folder",
    });
    expect(appFill({ mods_folder: { base: "game", path: "" } }).left).toEqual([{ field: "mods_folder", why: "path is empty" }]);
  });

  it("a JSON __proto__ field changes nothing", () => {
    const fields = JSON.parse('{"__proto__": {"title": "polluted"}, "summary": "ok"}') as Record<string, unknown>;
    const fill = appFill(fields);
    expect(fill.values).toEqual({ summary: "ok" });
    expect(({} as { title?: string }).title).toBeUndefined();
    expect(fill.unknown).toBe(1);
  });
});

describe("a People page's fill", () => {
  it("fills in every field the editor has", () => {
    const fill = peopleFill(PERSON, SKILLS);
    expect(fill.values).toEqual({
      ...PERSON,
      services: [{ id: "", ...PERSON.services[0] }],
      // Every section once; any not given follow in the usual order.
      sections: ["services", "portfolio", "reviews", "worked_on", "about"],
    });
    expect(fill.filled).toEqual(Object.keys(PERSON));
    expect(fill.left).toEqual([]);
    expect(fill.unknown).toBe(0);
  });

  it("never answers for the person: 18 or older, agreements, payments; business status is only a suggestion", () => {
    const fill = peopleFill({ name: "Sea", trader: true, confirm_adult: true, accept_terms: true, stripe: "acct_1" }, SKILLS);
    expect(fill.values).toEqual({ name: "Sea" });
    expect("trader" in fill.values).toBe(false);
    expect(fill.traderSuggested).toBe(true);
    // In the link's order.
    expect(fill.left).toEqual([
      { field: "confirm_adult", why: "only you can say you're 18 or older" },
      { field: "accept_terms", why: "accepting an agreement is yours to do" },
      { field: "stripe", why: "payments are set up by you, with Stripe" },
    ]);
    expect(peopleFill({ trader: "yes" }).traderSuggested).toBeUndefined();
  });

  it("keeps the good services and says what's wrong with the others", () => {
    const good = { name: "Pack", price_minor: 1000, currency: "usd", days: 1, skill: "sound" };
    const fill = peopleFill(
      {
        skills: ["sound"],
        services: [
          good,
          { ...good, price_minor: 999 },
          { ...good, currency: "jpy" },
          { ...good, days: 1.5 },
          { ...good, name: "" },
          { ...good, skill: "juggling" },
          "Pack",
          { ...good, description: "z".repeat(601) },
        ],
      },
      SKILLS,
    );
    expect(fill.values.services).toEqual([{ id: "", description: "", ...good }]);
    expect(fill.left).toEqual([
      {
        field: "services",
        why: [
          "service 2: price_minor should be a whole number from 1,000 to 1,000,000",
          "service 3: currency should be one of usd, eur, gbp",
          "service 4: days should be a whole number from 1 to 365",
          "service 5: name is empty",
          "service 6: skill isn't a skill",
          "service 7: should be an object",
          "service 8: description is over 600 characters",
        ].join("; "),
      },
    ]);
    expect(peopleFill({ services: Array.from({ length: 9 }, () => good) }).left).toEqual([{ field: "services", why: "has more than 8" }]);
  });

  it("a service's skill becomes one of theirs, since the editor offers services only for those", () => {
    const service = { name: "Sprites", price_minor: 5000, currency: "gbp", days: 7, skill: "pixel-art" };
    expect(peopleFill({ skills: ["music"], services: [service] }, SKILLS).values.skills).toEqual(["music", "pixel-art"]);
    const fill = peopleFill({ name: "Pix", services: [service, { ...service, name: "More" }] }, SKILLS);
    expect(fill.values.skills).toEqual(["pixel-art"]);
    expect(fill.filled).toEqual(["name", "skills", "services"]);
  });

  it("holds every field to the editor's limits and types", () => {
    const fill = peopleFill(
      {
        name: "n".repeat(61),
        handle: "Sea Sounds!",
        kind: "company",
        team_size: 1,
        headline: "h".repeat(141),
        skills: ["sound", "juggling"],
        ai_use: "some",
        ai_note: "a\u0007",
        location: 12,
        turnaround: "t".repeat(41),
        taking_work: "true",
        about_md: "a".repeat(6001),
        accent: "neon",
        sections: ["portfolio", "comments"],
        links: [{ label: "Site", url: "http://example.com" }, { url: "https://example.com" }, { label: "l".repeat(31), url: "https://x.example" }],
        portfolio_repo: "ftp://github.com/sea/portfolio",
      },
      SKILLS,
    );
    expect(fill.values).toEqual({ skills: ["sound"], links: [{ label: "", url: "https://example.com" }] });
    expect(Object.fromEntries(fill.left.map((l) => [l.field, l.why]))).toEqual({
      name: "is over 60 characters",
      handle: "should be 3 to 40 lowercase letters, numbers and single dashes",
      kind: "should be one of individual, studio",
      team_size: "should be a whole number from 2 to 500",
      headline: "is over 140 characters",
      skills: "juggling isn't one of the choices",
      ai_use: "should be one of none, assisted, heavy",
      ai_note: "should be one line",
      location: "should be text",
      turnaround: "is over 40 characters",
      taking_work: "should be true or false",
      about_md: "is over 6,000 characters",
      accent: "should be one of slime, ocean, grape, rose, ember, sun, forest, slate",
      sections: "comments isn't one of the choices",
      links: "link 1: url should be an https:// address; link 3: label is over 30 characters",
      portfolio_repo: "should be the repository's https:// address",
    });
  });

  it("an individual has no team size", () => {
    expect(peopleFill({ kind: "individual", team_size: 4 }).values).toEqual({ kind: "individual", team_size: null });
    expect(peopleFill({ kind: "studio", team_size: null }).values).toEqual({ kind: "studio", team_size: null });
  });
});

describe("keeping a fill through signing in", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    sessionStorage.clear();
    window.history.replaceState(null, "", "/");
  });

  it("takes it out of the address bar and keeps it in the tab until the form has it", () => {
    window.history.replaceState({ idx: 3 }, "", `/publish/new?x=1${link(APP)}`);
    expect(takeHandoff("app")).toEqual({ fields: APP });
    expect(window.location.hash).toBe("");
    expect(window.location.pathname + window.location.search).toBe("/publish/new?x=1");
    // The router's own state stays.
    expect(window.history.state).toEqual({ idx: 3 });
    // Back from signing in: still there, and only for its own form.
    expect(takeHandoff("app")).toEqual({ fields: APP });
    expect(takeHandoff("people")).toBeNull();
    forgetHandoff("app");
    expect(takeHandoff("app")).toBeNull();
  });

  it("isn't applied once it's two hours old", () => {
    window.history.replaceState(null, "", `/people/me${link(PERSON)}`);
    const now = Date.now();
    expect(takeHandoff("people", now)).toEqual({ fields: PERSON });
    expect(takeHandoff("people", now + KEEP_FOR_MS - 1)).toEqual({ fields: PERSON });
    expect(takeHandoff("people", now + KEEP_FOR_MS)).toBeNull();
    // And it's gone.
    expect(takeHandoff("people", now)).toBeNull();
  });

  it("a broken link replaces one kept before, and isn't kept itself", () => {
    window.history.replaceState(null, "", `/publish/new${link(APP)}`);
    takeHandoff("app");
    window.history.replaceState(null, "", "/publish/new#fill=%7Bbroken");
    expect(takeHandoff("app")).toEqual({ error: "what follows #fill= isn't JSON" });
    expect(window.location.hash).toBe("");
    expect(takeHandoff("app")).toBeNull();
  });

  it("damaged storage is dropped", () => {
    sessionStorage.setItem("slop.handoff.app", "{not json");
    expect(takeHandoff("app")).toBeNull();
    sessionStorage.setItem("slop.handoff.app", JSON.stringify({ at: Date.now(), fields: [1] }));
    expect(takeHandoff("app")).toBeNull();
    expect(sessionStorage.getItem("slop.handoff.app")).toBeNull();
  });

  it("works for this page view when storage is blocked", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "removeItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    window.history.replaceState(null, "", `/publish/new${link({ title: "Ferry" })}`);
    expect(takeHandoff("app")).toEqual({ fields: { title: "Ferry" } });
    expect(takeHandoff("app")).toBeNull();
    expect(() => forgetHandoff("app")).not.toThrow();
    expect(() => keepPortfolioRepo("https://github.com/sea/portfolio")).not.toThrow();
    expect(keptPortfolioRepo()).toBeUndefined();
  });

  it("a portfolio repository waits for the page to exist", () => {
    keepPortfolioRepo("https://github.com/sea/portfolio");
    expect(keptPortfolioRepo()).toBe("https://github.com/sea/portfolio");
    forgetPortfolioRepo();
    expect(keptPortfolioRepo()).toBeUndefined();
    const now = Date.now();
    keepPortfolioRepo("https://github.com/sea/portfolio", now - KEEP_FOR_MS);
    expect(keptPortfolioRepo(now)).toBeUndefined();
    sessionStorage.setItem("slop.handoff.portfolio", JSON.stringify({ at: now, url: "javascript:alert(1)" }));
    expect(keptPortfolioRepo(now)).toBeUndefined();
  });
});
