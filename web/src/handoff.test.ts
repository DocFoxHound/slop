// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  appFill,
  decodeFill,
  forgetHandoff,
  forgetPortfolioRepo,
  hireFill,
  keepPortfolioRepo,
  keptPortfolioRepo,
  KEEP_FOR_MS,
  peopleFill,
  requestFill,
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

  it("a game that plays in the browser: In the browser alone, and its play address", () => {
    const fill = appFill({ platforms: ["browser"], play_address: " https://games.example.com/play/ ", kind: "game" }, TAGS);
    expect(fill.values).toEqual({ kind: "game", platforms: ["browser"], play_address: "https://games.example.com/play/" });
    expect(fill.filled).toEqual(["kind", "platforms", "play_address"]);
    expect(fill.left).toEqual([]);

    // Never with Windows or Linux.
    const mixed = appFill({ platforms: ["browser", "windows"], play_address: "https://games.example.com/" });
    expect(mixed.values).toEqual({});
    expect(mixed.left).toEqual([
      { field: "platforms", why: "can't put browser with windows or linux: a browser game plays only in the browser" },
      { field: "play_address", why: 'only a game that plays in the browser has one: its platforms should be ["browser"]' },
    ]);
    // A play address for a download is left out; so are a browser game's sandbox mode and mods folder.
    expect(appFill({ platforms: ["windows"], play_address: "https://games.example.com/" }).values).toEqual({ platforms: ["windows"] });
    const extras = appFill({ platforms: ["browser"], sandbox_ready: true, mods_folder: { base: "game", path: "mods" } });
    expect(extras.values).toEqual({ platforms: ["browser"] });
    expect(extras.left.map((l) => l.why)).toEqual(["a browser game has no sandbox mode", "a browser game has no mods folder"]);
  });

  it("leaves In the browser out, saying why, for an app or a Private page", () => {
    const app = appFill({ platforms: ["browser"], kind: "app", play_address: "https://games.example.com/play/" });
    expect(app.values).toEqual({ kind: "app" });
    expect(app.left).toEqual([
      { field: "platforms", why: "only games can play in the browser for now" },
      { field: "play_address", why: "only games can play in the browser for now" },
    ]);
    const hidden = appFill({ platforms: ["browser"], kind: "game", visibility: "private", play_address: "https://games.example.com/play/" });
    expect(hidden.values).toEqual({ kind: "game", visibility: "private" });
    expect(hidden.left).toEqual([
      { field: "platforms", why: "a browser game's address is public on the web, so it can't be private" },
      { field: "play_address", why: "a browser game's address is public on the web, so it can't be private" },
    ]);
    // Its sandbox mode then stays, as for any download.
    expect(appFill({ platforms: ["browser"], kind: "app", sandbox_ready: true }).values).toEqual({ kind: "app", sandbox_ready: true });
    // A public game keeps it.
    expect(appFill({ platforms: ["browser"], kind: "game", visibility: "public" }).values.platforms).toEqual(["browser"]);
  });

  it("takes a play address only as https, with no user name or password, up to 2,000 characters", () => {
    const address = (play_address: string) => appFill({ platforms: ["browser"], play_address }).values.play_address;
    expect(address("http://games.example.com/")).toBeUndefined();
    expect(address("https://me:pw@games.example.com/")).toBeUndefined();
    expect(address("javascript:alert(1)")).toBeUndefined();
    expect(address(`https://games.example.com/${"p".repeat(2000)}`)).toBeUndefined();
    expect(appFill({ platforms: ["browser"], play_address: "ftp://x" }).left).toEqual([
      { field: "play_address", why: "should be an https:// address" },
    ]);
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

describe("filling in a request to hire someone", () => {
  // Vera's page (pixel art and music, one package) and the visitor's two projects.
  const VERA = {
    skills: ["pixel-art", "music"],
    services: [{ id: "0a1b2c3d", skill: "pixel-art" }],
    projects: ["cleos-tavern", "night-ferry"],
  };
  const REQUEST = {
    app_slug: "night-ferry",
    service_id: "0a1b2c3d",
    skill: "music",
    request_md: "1 ferry, 4 directions, 32×32.\nTransparent PNG.",
    wanted_by: "2030-01-31",
  };

  it("takes the form's fields, in the form's order", () => {
    const fill = hireFill(REQUEST, VERA);
    expect(fill.values).toEqual(REQUEST);
    expect(fill.filled).toEqual(["app_slug", "service_id", "skill", "request_md", "wanted_by"]);
    expect(fill.left).toEqual([]);
    expect(fill.unknown).toBe(0);
  });

  it("a package brings its skill when the link names none", () => {
    expect(hireFill({ service_id: "0a1b2c3d" }, VERA).values).toEqual({ service_id: "0a1b2c3d", skill: "pixel-art" });
    // A skill that isn't theirs is left out, and the package's stands.
    const fill = hireFill({ service_id: "0a1b2c3d", skill: "voice" }, VERA);
    expect(fill.values).toEqual({ service_id: "0a1b2c3d", skill: "pixel-art" });
    expect(fill.left).toEqual([{ field: "skill", why: "isn't one of their skills" }]);
  });

  it("never fills in what commits the person, and says so", () => {
    const fill = hireFill(
      {
        request_md: "A tavern cat.",
        exact_wording_ack: true,
        confirm_adult: true,
        client_is_business: true,
        inquiry: true,
        accept_terms: true,
        price_minor: 100,
      },
      VERA,
    );
    expect(fill.values).toEqual({ request_md: "A tavern cat." });
    expect(Object.fromEntries(fill.left.map((l) => [l.field, l.why]))).toEqual({
      exact_wording_ack: "only you can say you've understood it",
      confirm_adult: "only you can say you're 18 or older",
      client_is_business: "whether you're hiring as a business is yours to say",
      inquiry: "you choose between Send request and Ask about it first",
      accept_terms: "accepting an agreement is yours to do",
    });
    expect(fill.unknown).toBe(1);
  });

  it("leaves out what the form wouldn't take, with why", () => {
    const fill = hireFill(
      {
        app_slug: "someone-elses",
        service_id: "ffffffff",
        skill: ["pixel-art"],
        request_md: "x".repeat(4001),
        wanted_by: "2030-02-30",
      },
      VERA,
    );
    expect(fill.values).toEqual({});
    expect(Object.fromEntries(fill.left.map((l) => [l.field, l.why]))).toEqual({
      app_slug: "isn't one of your projects",
      service_id: "isn't one of their packages now",
      skill: "isn't one of their skills",
      request_md: "is over 4,000 characters",
      wanted_by: "should be a date, like 2030-01-31",
    });
    for (const bad of ["31/01/2030", "2030-1-31", "soon", 20300131, "2030-13-01", ""]) {
      expect(hireFill({ wanted_by: bad }, VERA).values).toEqual({});
    }
    expect(hireFill({ request_md: "  \n " }, VERA).left).toEqual([{ field: "request_md", why: "is empty" }]);
    expect(hireFill({ request_md: "a\u0000b" }, VERA).left).toEqual([{ field: "request_md", why: "has characters that aren't allowed" }]);
    // Names every object has aren't projects, packages or skills.
    expect(hireFill({ app_slug: "toString", skill: "constructor" }, VERA).values).toEqual({});
  });
});

describe("filling in an open request", () => {
  // Rhea's two projects in the store, and the skills People has.
  const RHEA = { projects: ["rheas-rooftops", "night-ferry"], skills: ["pixel-art", "music", "writing"] };
  const TODAY = "2030-01-15";
  const REQUEST = {
    app_slug: "night-ferry",
    skill: "music",
    title: "Main theme",
    description_md: "A 90-second loop for the title screen.\n\nCalm, with a version that fades out.",
    wanted_by: "2030-02-01",
  };

  it("takes the form's fields, in the form's order", () => {
    const fill = requestFill(REQUEST, RHEA, TODAY);
    expect(fill.values).toEqual(REQUEST);
    expect(fill.filled).toEqual(["app_slug", "skill", "title", "description_md", "wanted_by"]);
    expect(fill.left).toEqual([]);
    expect(fill.unknown).toBe(0);
    // Today itself is fine.
    expect(requestFill({ wanted_by: TODAY }, RHEA, TODAY).values).toEqual({ wanted_by: TODAY });
  });

  it("leaves out what the form wouldn't take, with why", () => {
    const fill = requestFill(
      {
        app_slug: "someone-elses",
        skill: "juggling",
        title: "x".repeat(101),
        description_md: "x".repeat(2001),
        wanted_by: "2030-01-14",
      },
      RHEA,
      TODAY,
    );
    expect(fill.values).toEqual({});
    expect(Object.fromEntries(fill.left.map((l) => [l.field, l.why]))).toEqual({
      app_slug: "isn't one of your projects in the store",
      skill: "isn't a skill on People",
      title: "is over 100 characters",
      description_md: "is over 2,000 characters",
      wanted_by: "is in the past",
    });
    expect(requestFill({ title: "Main\ntheme" }, RHEA, TODAY).left).toEqual([{ field: "title", why: "should be one line" }]);
    expect(requestFill({ title: "  " }, RHEA, TODAY).left).toEqual([{ field: "title", why: "is empty" }]);
    expect(requestFill({ description_md: "a\u0007b" }, RHEA, TODAY).left).toEqual([{ field: "description_md", why: "has characters that aren't allowed" }]);
    for (const bad of ["31/01/2030", "2030-1-31", "soon", 20300131, "2030-02-30", ""]) {
      expect(requestFill({ wanted_by: bad }, RHEA, TODAY).left).toEqual([{ field: "wanted_by", why: "should be a date, like 2030-01-31" }]);
    }
    // Exactly 100 and 2,000 go in.
    expect(requestFill({ title: "x".repeat(100), description_md: "x".repeat(2000) }, RHEA, TODAY).filled).toEqual(["title", "description_md"]);
    // Names every object has aren't projects or skills.
    expect(requestFill({ app_slug: "toString", skill: "constructor" }, RHEA, TODAY).values).toEqual({});
  });

  it("never fills in the 18+ tick, an agreement, a budget or a price, and says so; other fields are unknown", () => {
    const fill = requestFill(
      { title: "Main theme", confirm_adult: true, adult: true, accept_terms: true, budget: 500, price: 500, currency: "usd", deadline: "2030-02-01" },
      RHEA,
      TODAY,
    );
    expect(fill.values).toEqual({ title: "Main theme" });
    expect(Object.fromEntries(fill.left.map((l) => [l.field, l.why]))).toEqual({
      confirm_adult: "only you can say you're 18 or older",
      adult: "only you can say you're 18 or older",
      accept_terms: "accepting an agreement is yours to do",
      budget: "there's no budget on a request: the creative sets the price in their quote",
      price: "there's no budget on a request: the creative sets the price in their quote",
    });
    expect(fill.unknown).toBe(2);
  });

  it("is kept for its own form", () => {
    window.history.replaceState(null, "", `/people?view=requests${link(REQUEST)}`);
    expect(takeHandoff("request")).toEqual({ fields: REQUEST });
    expect(window.location.hash).toBe("");
    expect(window.location.pathname + window.location.search).toBe("/people?view=requests");
    expect(sessionStorage.getItem("slop.handoff.request")).toContain("Main theme");
    expect(takeHandoff("hire")).toBeNull();
    forgetHandoff("request");
    expect(takeHandoff("request")).toBeNull();
    window.history.replaceState(null, "", "/");
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

  it("a request to hire is kept for the page it was opened on", () => {
    window.history.replaceState(null, "", `/people/vera-pixels${link({ request_md: "A tavern cat." })}`);
    expect(takeHandoff("hire", undefined, "vera-pixels")).toEqual({ fields: { request_md: "A tavern cat." } });
    expect(window.location.hash).toBe("");
    // Someone else's page doesn't get it, and doesn't lose it for Vera's.
    expect(takeHandoff("hire", undefined, "olly-sounds")).toBeNull();
    expect(takeHandoff("hire", undefined, "vera-pixels")).toEqual({ fields: { request_md: "A tavern cat." } });
    // Nor does the People editor, which keeps its own.
    expect(takeHandoff("people")).toBeNull();
    forgetHandoff("hire");
    expect(takeHandoff("hire", undefined, "vera-pixels")).toBeNull();
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
