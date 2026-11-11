// SPDX-License-Identifier: AGPL-3.0-or-later
// Open requests on People (PeopleRequests.tsx): the board's one order, who is offered "Post a
// request", the form, who can put their name forward, the maker's list of names and its "Ask … for
// a quote", and the hire form's words when its fill came from the person's own request.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  api,
  ApiError,
  type AppSummary,
  type CreativeCard,
  type CreativePage,
  type Me,
  type MyProfile,
  type MyWorkRequests,
  type WorkRequest,
  type WorkRequestName,
  type WorkRequestOwner,
} from "../api";
import { HandoffNotice } from "../components/Handoff";
import type { Fill } from "../handoff";
import { People, PersonPage } from "./People";

const session = vi.hoisted(() => ({ me: null as Partial<Me> | null }));
vi.mock("../session", () => ({ useSession: () => ({ me: session.me, refresh: async () => {}, signOut: async () => {} }) }));

// jsdom has no modal dialogs; the Report and Terms dialogs only need to open and close.
HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
  this.open = true;
};
HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
  this.open = false;
};

const SKILLS = [
  { slug: "pixel-art", name: "Pixel Art" },
  { slug: "music", name: "Music" },
  { slug: "writing", name: "Writing" },
];
const ROOFTOPS = { slug: "rheas-rooftops", title: "Rhea's Rooftops" };
const idOf = (n: number) => `00000000-0000-7000-8000-${String(n).padStart(12, "0")}`;
const req = (n: number, over: Partial<WorkRequest> = {}): WorkRequest => ({
  id: idOf(n),
  title: `Request ${n}`,
  description_md: `Text ${n}`,
  skill: { slug: "pixel-art", name: "Pixel Art" },
  wanted_by: null,
  project: ROOFTOPS,
  maker: "Rhea Requester",
  created_at: "2026-11-10T09:00:00Z",
  expires_at: "2026-12-10T09:00:00Z",
  mine: false,
  put_forward: null,
  owner: null,
  ...over,
});
const owner = (over: Partial<WorkRequestOwner> = {}): WorkRequestOwner => ({
  status: "open",
  on_board: true,
  ended_at: null,
  hidden_reason: null,
  responses: [],
  responses_total: 0,
  ...over,
});
const posted = (n: number, o: Partial<WorkRequestOwner> = {}) => ({ ...req(n, { mine: true }), owner: owner(o) });
const myRequests = (over: Partial<MyWorkRequests> = {}): MyWorkRequests => ({
  posted: [],
  put_forward: [],
  projects: [],
  has_apps: false,
  max_open: 3,
  ...over,
});
const card = (handle: string, name: string, over: Partial<CreativeCard> = {}): CreativeCard => ({
  handle,
  name,
  kind: "individual",
  team_size: null,
  headline: "Pixel art",
  location: "",
  skills: [{ slug: "pixel-art", name: "Pixel Art" }],
  ai_use: "none",
  taking_work: true,
  can_hire: true,
  accent: "grape",
  avatar_url: null,
  preview: [],
  from_price: null,
  rating: { not_slop: 0, slop: 0, percent: null, label: null, min_reviews: 3 },
  clients: 0,
  ...over,
});
const name = (person: CreativeCard, at: string, service: WorkRequestName["service"] = null): WorkRequestName => ({ person, service, at });
const SPRITES = { id: "a1b2c3d4", name: "Sprite sheet", description: "", price_minor: 12000, currency: "usd" as const, days: 7, skill: "pixel-art" };
const profile = (over: Partial<MyProfile> = {}): MyProfile => ({
  handle: "vera-pixels",
  name: "Vera Okafor",
  kind: "individual",
  team_size: null,
  headline: "",
  about_md: "",
  location: "",
  skills: ["pixel-art", "music"],
  ai_use: "none",
  ai_note: "",
  trader: false,
  taking_work: true,
  turnaround: "",
  services: [SPRITES, { ...SPRITES, id: "e5f6a7b8", name: "Theme tune", skill: "music" }],
  accent: "grape",
  sections: ["services", "portfolio"],
  links: [],
  portfolio_repo: null,
  stripe_status: "active",
  live: true,
  hidden_reason: null,
  payments: "stripe",
  status: { state: "live", reason: null, update_waiting: false, waiting_since: null, problem: null, suspended: false },
  ...over,
});

/** Where the router went, for "Ask … for a quote". */
function Where() {
  const loc = useLocation();
  return <output aria-label="Where the router went">{JSON.stringify({ pathname: loc.pathname, hash: loc.hash, state: loc.state })}</output>;
}
const where = () => JSON.parse(screen.getByRole("status", { name: "Where the router went" }).textContent ?? "{}");

const show = (path = "/people?view=requests") =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/people" element={<People />} />
        <Route path="/people/:handle" element={<Where />} />
      </Routes>
    </MemoryRouter>,
  );
const detail = (n: number) => `/people?view=requests&request=${idOf(n)}`;
/** How long a wait may take after a list of 50 to 200 cards is drawn (slow in jsdom, more so under load). */
const LONG = 10_000;
const signedIn = (adult_confirmed = true) => {
  session.me = { id: "u1", display_name: "Rhea Requester", adult_confirmed };
};

beforeEach(() => {
  session.me = null;
  vi.spyOn(api.people, "skills").mockResolvedValue(SKILLS);
  vi.spyOn(api.people, "mine").mockRejectedValue(new ApiError(404, "not_found", "not found"));
  vi.spyOn(api.people.requests, "list").mockResolvedValue([]);
  vi.spyOn(api.people.requests, "mine").mockResolvedValue(myRequests());
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  sessionStorage.clear();
  window.history.replaceState(null, "", "/");
});

describe("the board", () => {
  it("says its one order, and shows the cards in the order the API gives with nothing about any creative", async () => {
    const list = vi.mocked(api.people.requests.list).mockImplementation(async (q = {}) =>
      q.skill === "music" ? [] : [req(1, { wanted_by: "2026-12-20" }), req(2), req(3, { skill: { slug: "music", name: "Music" } })],
    );
    show();
    const board = await screen.findByRole("region", { name: "Open requests" });
    expect(within(board).getByText(/^Newest first\. Nothing else changes the order/)).toBeTruthy();
    expect(within(board).getByText(/There's no budget on a request: the creative sets the price in their quote\./)).toBeTruthy();
    const items = await within(board).findAllByRole("listitem");
    expect(items.map((li) => within(li).getByRole("heading", { level: 3 }).textContent)).toEqual(["Request 1", "Request 2", "Request 3"]);
    expect(within(items[0]).getByRole("link", { name: "Request 1" }).getAttribute("href")).toBe(detail(1));
    expect(within(items[0]).getByRole("link", { name: "Rhea's Rooftops" }).getAttribute("href")).toBe("/app/rheas-rooftops");
    expect(items[0].textContent).toContain("Pixel Art · for Rhea's Rooftops · posted by Rhea Requester on Nov 10, 2026");
    expect(within(items[0]).getByText("Hoping to have it by Dec 20, 2026")).toBeTruthy();
    // No count, no price, no badge: nothing on a card is about the creatives.
    for (const li of items) expect(li.textContent).not.toMatch(/name forward|\$|budget|YOUR REQUEST/i);
    expect(list).toHaveBeenCalledWith({ skill: undefined });
    // The skill buttons filter it; pressed again, the filter goes.
    const skills = screen.getByRole("group", { name: "Skill" });
    fireEvent.click(within(skills).getByRole("button", { name: "Music" }));
    expect(await screen.findByText("No open requests for Music right now.")).toBeTruthy();
    expect(screen.getByRole("heading", { level: 2, name: "Open requests: Music" })).toBeTruthy();
    expect(within(skills).getByRole("button", { name: "Music" }).getAttribute("aria-pressed")).toBe("true");
    expect(list).toHaveBeenLastCalledWith({ skill: "music" });
    fireEvent.click(within(skills).getByRole("button", { name: "Music" }));
    await waitFor(() => expect(list).toHaveBeenLastCalledWith({ skill: undefined }));
    expect(await screen.findByRole("link", { name: "Request 2" })).toBeTruthy();
  });

  it("marks the viewer's own requests and the ones they put their name forward for, and only those", async () => {
    signedIn();
    vi.mocked(api.people.requests.list).mockResolvedValue([req(1, { mine: true }), req(2, { put_forward: { service_id: null, at: "2026-11-10T10:00:00Z" } }), req(3)]);
    show();
    const items = await within(await screen.findByRole("region", { name: "Open requests" })).findAllByRole("listitem");
    expect(within(items[0]).getByText("YOUR REQUEST")).toBeTruthy();
    expect(within(items[1]).getByText("YOU PUT YOUR NAME FORWARD")).toBeTruthy();
    expect(items[2].querySelector(".badge")).toBeNull();
  });

  it("shows more, 50 at a time, and the focus goes to the first new one", async () => {
    const list = vi.mocked(api.people.requests.list).mockImplementation(async (q = {}) =>
      q.offset ? [req(51)] : Array.from({ length: 50 }, (_, i) => req(i + 1)),
    );
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Show more requests" }, { timeout: LONG }));
    // Role queries over 51 cards are slow in jsdom: find the new link once, then wait for the focus.
    const added = await screen.findByRole("link", { name: "Request 51" }, { timeout: LONG });
    await waitFor(() => expect(document.activeElement).toBe(added), { timeout: LONG });
    expect(list).toHaveBeenLastCalledWith({ skill: undefined, offset: 50 });
    expect(screen.getAllByRole("heading", { level: 3 })).toHaveLength(51);
    // The last page had fewer than 50: no more to show.
    expect(screen.queryByRole("button", { name: "Show more requests" })).toBeNull();
  }, 30_000);

  it("says when nothing is open, and offers to try again when it can't load", async () => {
    show();
    expect(await screen.findByText("No open requests right now.")).toBeTruthy();
    cleanup();
    vi.mocked(api.people.requests.list).mockRejectedValue(new ApiError(500, "internal", "Something broke."));
    show();
    expect(await screen.findByText("Something broke.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });
});

describe("People's tabs", () => {
  it("the tab pressed keeps the focus while the view changes under it", async () => {
    vi.spyOn(api.people, "list").mockResolvedValue({ people: [], skills: [] });
    vi.spyOn(api.people, "discover").mockResolvedValue({ taking_work: [], new: [], studios: [], by_skill: [], suggested: null });
    show();
    await screen.findByText("No open requests right now.");
    const tabs = screen.getByRole("tablist", { name: "People and open requests" });
    const people = within(tabs).getByRole("tab", { name: "People" });
    people.focus();
    fireEvent.click(people);
    expect(await screen.findByRole("heading", { level: 2, name: "Everyone" })).toBeTruthy();
    expect(document.activeElement).toBe(people);
    expect(people.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("heading", { level: 1, name: "People" })).toBeTruthy();
    const requests = within(tabs).getByRole("tab", { name: "Open requests" });
    requests.focus();
    fireEvent.click(requests);
    expect(await screen.findByText("No open requests right now.")).toBeTruthy();
    expect(document.activeElement).toBe(requests);
    expect(requests.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("heading", { level: 1, name: "Open requests" })).toBeTruthy();
  });
});

describe("your requests", () => {
  /** Waits for the page to have its answers, so "not shown" is a real answer. */
  async function settled() {
    await screen.findByText("No open requests right now.");
    await waitFor(() => expect(api.people.requests.mine).toHaveBeenCalled());
    await act(async () => {});
  }

  it("offers Post a request only with a project in the store and fewer than three open", async () => {
    signedIn();
    const mine = vi.mocked(api.people.requests.mine);
    // No app at all: nothing, and no explanation.
    show();
    await settled();
    expect(screen.queryByRole("region", { name: "Your requests" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Post a request" })).toBeNull();
    cleanup();

    // An app, but none in the store (a draft): why, and no button.
    mine.mockResolvedValue(myRequests({ has_apps: true }));
    show();
    await settled();
    const yours = screen.getByRole("region", { name: "Your requests" });
    expect(within(yours).getByText("A request is for a project whose page is in the store.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Post a request" })).toBeNull();
    cleanup();

    // Three open: one sentence instead of the button, and the three listed.
    const three = [posted(1), posted(2), posted(3, { on_board: false })];
    mine.mockResolvedValue(myRequests({ projects: [ROOFTOPS], has_apps: true, posted: three }));
    show();
    await settled();
    expect(screen.getByText("You have 3 open requests, the most at one time. Close one to post another.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Post a request" })).toBeNull();
    const cards = within(screen.getByRole("region", { name: "Your requests" })).getAllByRole("listitem");
    expect(cards[0].textContent).toContain("Open until Dec 10, 2026");
    expect(cards[0].textContent).toContain("Nobody has put their name forward yet");
    expect(cards[2].textContent).toContain("Not showing on the board: its project isn't in the store right now.");
    cleanup();

    // Two open, one ended: the button, and each state said.
    mine.mockResolvedValue(
      myRequests({
        projects: [ROOFTOPS],
        has_apps: true,
        posted: [
          posted(1, { responses_total: 2 }),
          posted(2, { status: "closed", on_board: false, ended_at: "2026-11-11T10:00:00Z" }),
          posted(3, { status: "hidden", on_board: false, hidden_reason: "Spam" }),
          posted(4, { status: "expired", on_board: false, ended_at: "2026-11-09T10:00:00Z" }),
        ],
      }),
    );
    show();
    await settled();
    const region = screen.getByRole("region", { name: "Your requests" });
    // Their own heading, and the cards below it.
    expect(within(region).getByRole("heading", { level: 3, name: "Requests you posted" })).toBeTruthy();
    expect(within(region).getAllByRole("heading", { level: 4 }).map((h) => h.textContent)).toEqual(["Request 1", "Request 2", "Request 3", "Request 4"]);
    const ended = within(region).getAllByRole("listitem");
    expect(ended[0].textContent).toContain("2 put their name forward");
    expect(ended[1].textContent).toContain("Closed on Nov 11, 2026");
    expect(ended[2].textContent).toContain("Hidden by SLAPorSLOP. See the decision");
    expect(within(ended[2]).getByRole("link", { name: "See the decision" }).getAttribute("href")).toBe("/account#decisions");
    expect(ended[3].textContent).toContain("Ran out on Nov 9, 2026");
    fireEvent.click(screen.getByRole("button", { name: "Post a request" }));
    expect(screen.getByRole("form", { name: "Post a request" })).toBeTruthy();
  });

  it("lists the requests they put their name forward for", async () => {
    signedIn();
    vi.mocked(api.people.requests.mine).mockResolvedValue(myRequests({ put_forward: [req(7, { put_forward: { service_id: null, at: "2026-11-10T10:00:00Z" } })] }));
    show();
    const yours = await screen.findByRole("region", { name: "Your requests" });
    expect(within(yours).getByRole("heading", { level: 3, name: "Requests you put your name forward for" })).toBeTruthy();
    expect(within(yours).getByRole("heading", { level: 4, name: "Request 7" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Post a request" })).toBeNull();
  });

  it("the form has no budget, says the request is public, asks 18+ only when not confirmed, and shows the API's refusal", async () => {
    signedIn(false);
    vi.mocked(api.people.requests.mine).mockResolvedValue(myRequests({ projects: [ROOFTOPS], has_apps: true }));
    const post = vi
      .spyOn(api.people.requests, "post")
      .mockRejectedValueOnce(new ApiError(409, "conflict", "you still owe Vera Okafor $120.00 for finished work; pay it from your jobs before posting a request"));
    show("/people?view=requests&skill=music");
    fireEvent.click(await screen.findByRole("button", { name: "Post a request" }));
    const form = screen.getByRole("form", { name: "Post a request" });
    expect(within(form).queryByRole("spinbutton")).toBeNull();
    expect(within(form).queryByRole("textbox", { name: /budget|price/i })).toBeNull();
    expect(within(form).getByText("There's no budget on a request: the creative sets the price in their quote.")).toBeTruthy();
    expect(
      within(form).getByText(
        "Anyone can read this request, with your display name and your project. It stays up for 30 days unless you close it sooner.",
      ),
    ).toBeTruthy();
    // Their one project, and the board's skill, are chosen already.
    expect((within(form).getByRole("combobox", { name: "For which project?" }) as HTMLSelectElement).value).toBe("rheas-rooftops");
    expect((within(form).getByRole("combobox", { name: "Skill" }) as HTMLSelectElement).value).toBe("music");
    const title = within(form).getByRole("textbox", { name: "Title" }) as HTMLInputElement;
    expect(title.maxLength).toBe(100);
    fireEvent.change(title, { target: { value: "Main theme" } });
    const text = within(form).getByRole("textbox", { name: /^What do you need\?/ }) as HTMLTextAreaElement;
    expect(text.maxLength).toBe(2000);
    fireEvent.change(text, { target: { value: "A loop." } });
    const date = within(form).getByLabelText(/Hoping to have it by/) as HTMLInputElement;
    expect(date.min).toBe(new Date().toISOString().slice(0, 10));
    fireEvent.click(within(form).getByRole("checkbox", { name: "I'm 18 or older" }));
    fireEvent.submit(form);
    expect((await within(form).findByRole("alert")).textContent).toBe(
      "you still owe Vera Okafor $120.00 for finished work; pay it from your jobs before posting a request",
    );
    expect(post).toHaveBeenCalledWith({
      app_slug: "rheas-rooftops",
      skill: "music",
      title: "Main theme",
      description_md: "A loop.",
      wanted_by: null,
      confirm_adult: true,
    });

    // A refusal about one field marks it and takes the focus there.
    post.mockRejectedValueOnce(new ApiError(422, "invalid", "one line", "title"));
    fireEvent.submit(form);
    await waitFor(() => expect(document.activeElement).toBe(title));
    expect(title.getAttribute("aria-invalid")).toBe("true");
    expect(title.getAttribute("aria-describedby")).toBe("request-form-error");

    // Posted: the form closes, and the line that says so takes the focus.
    post.mockResolvedValueOnce({ ...req(9, { mine: true }), owner: owner() });
    fireEvent.submit(form);
    const done = await screen.findByText("Your request is on the board.");
    expect(done.getAttribute("role")).toBe("status");
    await waitFor(() => expect(document.activeElement).toBe(done));
    expect(screen.queryByRole("form", { name: "Post a request" })).toBeNull();
    cleanup();

    // Confirmed already: no tick.
    signedIn(true);
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Post a request" }));
    expect(screen.queryByRole("checkbox", { name: "I'm 18 or older" })).toBeNull();
  });

  it("an assistant's prepared request opens the form filled in, never with the 18+ tick or a budget", async () => {
    signedIn(false);
    vi.mocked(api.people.requests.mine).mockResolvedValue(myRequests({ projects: [ROOFTOPS, { slug: "night-ferry", title: "Night Ferry" }], has_apps: true }));
    const fields = { app_slug: "night-ferry", skill: "writing", title: "Store page text", description_md: "Five lines.", budget: 500, confirm_adult: true };
    window.history.replaceState(null, "", `/people?view=requests#fill=${encodeURIComponent(JSON.stringify(fields))}`);
    const post = vi.spyOn(api.people.requests, "post");
    show();
    const notice = await screen.findByRole("status", { name: "Filled in by your AI assistant" });
    expect(notice.textContent).toContain("Check everything, especially the title and what you need, since both are public, then press Post request.");
    expect(within(notice).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Budget: there's no budget on a request: the creative sets the price in their quote",
      "I'm 18 or older: only you can say you're 18 or older",
    ]);
    const form = screen.getByRole("form", { name: "Post a request" });
    // The notice shows with the fill; the values go in right after.
    await waitFor(() => expect((within(form).getByRole("combobox", { name: "For which project?" }) as HTMLSelectElement).value).toBe("night-ferry"));
    expect((within(form).getByRole("combobox", { name: "Skill" }) as HTMLSelectElement).value).toBe("writing");
    expect((within(form).getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("Store page text");
    expect((within(form).getByRole("checkbox", { name: "I'm 18 or older" }) as HTMLInputElement).checked).toBe(false);
    expect(window.location.hash).toBe("");
    expect(post).not.toHaveBeenCalled();
    // In the form now: not kept for a reload.
    expect(sessionStorage.getItem("slop.handoff.request")).toBeNull();
    fireEvent.click(within(notice).getByRole("button", { name: "Clear the form" }));
    expect((within(form).getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("");
    expect(screen.queryByRole("status", { name: "Filled in by your AI assistant" })).toBeNull();
    // The button pressed is gone with the notice: the focus goes to the first field.
    expect(document.activeElement).toBe(within(form).getByRole("combobox", { name: "For which project?" }));
  });

  it("the focus goes into the form when it opens, and back to its button when it's cancelled", async () => {
    signedIn();
    vi.mocked(api.people.requests.mine).mockResolvedValue(myRequests({ projects: [ROOFTOPS], has_apps: true }));
    show();
    const open = await screen.findByRole("button", { name: "Post a request" });
    open.focus();
    fireEvent.click(open);
    const form = screen.getByRole("form", { name: "Post a request" });
    await waitFor(() => expect(document.activeElement).toBe(within(form).getByRole("heading", { level: 3, name: "Post a request" })));
    fireEvent.click(within(form).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("form", { name: "Post a request" })).toBeNull();
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("button", { name: "Post a request" })));
  });

  it("once posted, a prepared request doesn't open the form again, also after a look at one request and back", async () => {
    signedIn();
    vi.mocked(api.people.requests.mine).mockResolvedValue(myRequests({ projects: [ROOFTOPS], has_apps: true }));
    vi.mocked(api.people.requests.list).mockResolvedValue([req(1)]);
    vi.spyOn(api.people.requests, "one").mockResolvedValue(req(1));
    vi.spyOn(api.people.requests, "post").mockResolvedValue({ ...req(9, { mine: true }), owner: owner() });
    const fields = { skill: "music", title: "Main theme", description_md: "A loop." };
    window.history.replaceState(null, "", `/people?view=requests#fill=${encodeURIComponent(JSON.stringify(fields))}`);
    show();
    const form = await screen.findByRole("form", { name: "Post a request" });
    await waitFor(() => expect((within(form).getByRole("textbox", { name: "Title" }) as HTMLInputElement).value).toBe("Main theme"));
    fireEvent.submit(form);
    expect(await screen.findByText("Your request is on the board.")).toBeTruthy();
    fireEvent.click(screen.getByRole("link", { name: "Request 1" }));
    await screen.findByRole("heading", { level: 2, name: "Request 1" });
    fireEvent.click(screen.getByRole("link", { name: "Open requests" }));
    expect(await screen.findByRole("button", { name: "Post a request" })).toBeTruthy();
    expect(screen.queryByRole("form", { name: "Post a request" })).toBeNull();
  });

  it("signed out, a prepared request waits for signing in", async () => {
    window.history.replaceState(null, "", `/people?view=requests#fill=${encodeURIComponent(JSON.stringify({ title: "Main theme" }))}`);
    show();
    expect((await screen.findByText(/Your AI assistant's draft is ready\./)).closest("[role=status]")).toBeTruthy();
    expect(screen.getByRole("link", { name: "Sign in" }).getAttribute("href")).toBe(`/signin?return_to=${encodeURIComponent("/people?view=requests")}`);
    expect(sessionStorage.getItem("slop.handoff.request")).toContain("Main theme");
  });
});

describe("one request", () => {
  it("a 404 says it isn't open any more", async () => {
    vi.spyOn(api.people.requests, "one").mockRejectedValue(new ApiError(404, "not_found", "not found"));
    show(detail(1));
    expect(await screen.findByText("This request isn't open any more.")).toBeTruthy();
    expect(screen.getByRole("link", { name: "See open requests" }).getAttribute("href")).toBe("/people?view=requests");
  });

  it("shows the whole text through the Markdown renderer, with no budget, and a sign-in for visitors", async () => {
    vi.spyOn(api.people.requests, "one").mockResolvedValue(
      req(1, { description_md: "**Twelve** tiles.\n\n<script>alert(1)</script>\n\n# Big", wanted_by: "2026-12-20" }),
    );
    const { container } = show(detail(1));
    const title = await screen.findByRole("heading", { level: 2, name: "Request 1" });
    await waitFor(() => expect(document.activeElement).toBe(title));
    const article = screen.getByRole("article", { name: "Request 1" });
    expect(article.querySelector(".prose strong")?.textContent).toBe("Twelve");
    expect(container.querySelector("script")).toBeNull();
    // Headings in the text start below the request's own.
    expect(article.querySelector(".prose h3")?.textContent).toBe("Big");
    expect(article.textContent).toContain("open until Dec 10, 2026");
    expect(within(article).getByText("Hoping to have it by Dec 20, 2026")).toBeTruthy();
    expect(within(article).getByText("There's no budget on a request: the creative sets the price in their quote.")).toBeTruthy();
    expect(within(article).getByText("Have a People page? Sign in to put your name forward.")).toBeTruthy();
    expect(within(article).getByRole("link", { name: "Sign in" }).getAttribute("href")).toBe(
      `/signin?return_to=${encodeURIComponent(detail(1))}`,
    );
    expect(within(article).getByRole("button", { name: /Report/ })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Open requests" }).getAttribute("href")).toBe("/people?view=requests");
  });

  it("a creative who can be hired and lists the skill puts their name forward, naming a package, and takes it back", async () => {
    signedIn();
    vi.mocked(api.people.mine).mockResolvedValue(profile());
    const one = vi.spyOn(api.people.requests, "one").mockResolvedValue(req(1));
    const put = vi.spyOn(api.people.requests, "putForward").mockResolvedValue({ service_id: "a1b2c3d4", at: "2026-11-10T10:00:00Z" });
    const take = vi.spyOn(api.people.requests, "takeBack").mockResolvedValue(undefined);
    show(detail(1));
    const form = await screen.findByRole("form", { name: "Put your name forward" });
    const pkg = within(form).getByRole("combobox", { name: "Name one of your packages (optional)" }) as HTMLSelectElement;
    // Only her packages for this request's skill.
    expect([...pkg.options].map((o) => o.text)).toEqual(["No package", "Sprite sheet"]);
    expect(within(form).getByText(/Nothing else is sent, and you can take it back\./)).toBeTruthy();
    fireEvent.change(pkg, { target: { value: "a1b2c3d4" } });
    one.mockResolvedValue(req(1, { put_forward: { service_id: "a1b2c3d4", at: "2026-11-10T10:00:00Z" } }));
    fireEvent.submit(form);
    const said = await screen.findByText(
      "You put your name forward on Nov 10, 2026, naming your package “Sprite sheet”. The maker sees a link to your People page.",
    );
    expect(put).toHaveBeenCalledWith(idOf(1), "a1b2c3d4");
    await waitFor(() => expect(document.activeElement).toBe(said));
    expect(screen.queryByRole("form", { name: "Put your name forward" })).toBeNull();

    one.mockResolvedValue(req(1));
    fireEvent.click(screen.getByRole("button", { name: "Take my name back" }));
    const back = await screen.findByText("You took your name back.");
    expect(take).toHaveBeenCalledWith(idOf(1));
    await waitFor(() => expect(document.activeElement).toBe(back));
    expect(await screen.findByRole("form", { name: "Put your name forward" })).toBeTruthy();
  });

  it("a creative whose page isn't showing is told the maker doesn't see their name, and can still take it back", async () => {
    signedIn();
    vi.mocked(api.people.mine).mockResolvedValue(
      profile({ hidden_reason: "Spam", status: { ...profile().status, state: "hidden", reason: "Spam" } }),
    );
    vi.spyOn(api.people.requests, "one").mockResolvedValue(req(1, { put_forward: { service_id: null, at: "2026-11-10T10:00:00Z" } }));
    show(detail(1));
    expect(
      await screen.findByText("You put your name forward on Nov 10, 2026. Your People page isn't showing right now, so the maker doesn't see your name."),
    ).toBeTruthy();
    expect(screen.queryByText(/The maker sees a link to your People page/)).toBeNull();
    expect(screen.getByRole("button", { name: "Take my name back" })).toBeTruthy();
  });

  it("a creative who can't yet is told what to fix; someone with no page sees nothing", async () => {
    signedIn();
    const mine = vi.mocked(api.people.mine).mockResolvedValue(profile());
    const one = vi.spyOn(api.people.requests, "one").mockResolvedValue(req(2, { skill: { slug: "writing", name: "Writing" } }));
    show(detail(2));
    expect(await screen.findByText(/^Your People page doesn't list Writing\./)).toBeTruthy();
    expect(screen.getByRole("link", { name: "Add it to your page" }).getAttribute("href")).toBe("/people/me");
    expect(screen.queryByRole("button", { name: "Put my name forward" })).toBeNull();
    cleanup();

    one.mockResolvedValue(req(1));
    mine.mockResolvedValue(profile({ stripe_status: "none" }));
    show(detail(1));
    expect(await screen.findByText(/^Your People page can't be hired right now, so you can't put your name forward yet\./)).toBeTruthy();
    expect(screen.getByRole("link", { name: "See what it needs" }).getAttribute("href")).toBe("/people/me");
    expect(screen.queryByRole("button", { name: "Put my name forward" })).toBeNull();
    cleanup();

    // A page that isn't live yet, and no page at all: nothing to explain.
    const pages = [
      async () => profile({ status: { ...profile().status, state: "waiting" } }),
      async (): Promise<MyProfile> => {
        throw new ApiError(404, "not_found", "not found");
      },
    ];
    for (const page of pages) {
      mine.mockImplementation(page);
      show(detail(1));
      await screen.findByRole("heading", { level: 2, name: "Request 1" });
      await waitFor(() => expect(mine).toHaveBeenCalled());
      await act(async () => {});
      expect(screen.queryByRole("button", { name: "Put my name forward" })).toBeNull();
      expect(screen.queryByText(/Your People page (can't|doesn't)/)).toBeNull();
      expect(screen.queryByText(/Sign in to put your name forward/)).toBeNull();
      cleanup();
    }
  });

  it("the maker sees who put their name forward in the order received, and can close the request", async () => {
    signedIn();
    const wren = card("wren-sounds", "Wren Lee", { can_hire: false, taking_work: false });
    // Vera has the better rating and more clients: still second, as she came second.
    const vera = card("vera-pixels", "Vera Okafor", { rating: { not_slop: 9, slop: 1, percent: 90, label: "Really Slaps", min_reviews: 3 }, clients: 10 });
    const mill = card("pixel-mill", "Pixel Mill", { kind: "studio", team_size: 4 });
    const r = req(1, {
      mine: true,
      owner: owner({
        responses: [
          name(wren, "2026-11-10T10:00:00Z"),
          name(vera, "2026-11-10T11:00:00Z", { id: "a1b2c3d4", name: "Sprite sheet" }),
          name(mill, "2026-11-10T12:00:00Z"),
        ],
        responses_total: 3,
      }),
    });
    const one = vi.spyOn(api.people.requests, "one").mockResolvedValue(r);
    const close = vi.spyOn(api.people.requests, "close").mockResolvedValue(r);
    show(detail(1));
    const names = await screen.findByRole("region", { name: "Who put their name forward" });
    expect(within(names).getByText("In the order they did. SLAPorSLOP doesn't rank or recommend anyone here.")).toBeTruthy();
    const list = names.querySelector("ol")!;
    const items = within(list).getAllByRole("listitem");
    expect(items.map((li) => li.querySelector(".person-who b")?.textContent)).toEqual(["Wren Lee", "Vera Okafor", "Pixel Mill"]);
    expect(within(items[0]).queryByRole("button")).toBeNull();
    expect(within(items[0]).getByText("Wren Lee isn't taking paid work right now.")).toBeTruthy();
    expect(within(items[1]).getByText("Put forward on Nov 10, 2026 · Package: Sprite sheet")).toBeTruthy();
    expect(within(items[1]).getByRole("button", { name: "Ask Vera for a quote" })).toBeTruthy();
    expect(within(items[2]).getByRole("button", { name: "Ask them for a quote" })).toBeTruthy();
    expect(screen.getByText("Open until Dec 10, 2026.")).toBeTruthy();
    // The maker's own: no report, nothing to put forward.
    expect(screen.queryByRole("button", { name: /Report/ })).toBeNull();
    expect(screen.queryByRole("form", { name: "Put your name forward" })).toBeNull();

    one.mockResolvedValue({ ...r, owner: { ...r.owner!, status: "closed", on_board: false, ended_at: "2026-11-11T10:00:00Z" } });
    fireEvent.click(screen.getByRole("button", { name: "Close request" }));
    const closed = await screen.findByText("Request closed.");
    expect(close).toHaveBeenCalledWith(idOf(1));
    await waitFor(() => expect(document.activeElement).toBe(closed));
    expect(await screen.findByText("You closed this request on Nov 11, 2026.")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Close request" })).toBeNull();
  });

  it("Ask … for a quote opens that creative's hire form with exactly the request's fields and where they came from", async () => {
    signedIn();
    const vera = card("vera-pixels", "Vera Okafor");
    const mill = card("pixel-mill", "Pixel Mill", { kind: "studio", team_size: 4 });
    const r = (wanted_by: string) =>
      req(1, {
        mine: true,
        title: "Rooftop tiles",
        description_md: "Twelve tiles, 32×32.",
        wanted_by,
        owner: owner({ responses: [name(vera, "2026-11-10T11:00:00Z", { id: "a1b2c3d4", name: "Sprite sheet" }), name(mill, "2026-11-10T12:00:00Z")], responses_total: 2 }),
      });
    const one = vi.spyOn(api.people.requests, "one").mockResolvedValue(r("2099-01-31"));
    show(detail(1));
    fireEvent.click(await screen.findByRole("button", { name: "Ask Vera for a quote" }));
    const went = where();
    expect(went.pathname).toBe("/people/vera-pixels");
    expect(went.state).toEqual({ fromRequest: true });
    expect(went.hash.startsWith("#fill=")).toBe(true);
    const fields = JSON.parse(decodeURIComponent(went.hash.slice("#fill=".length)));
    expect(fields).toEqual({
      app_slug: "rheas-rooftops",
      skill: "pixel-art",
      request_md: "Rooftop tiles\n\nTwelve tiles, 32×32.",
      wanted_by: "2099-01-31",
      service_id: "a1b2c3d4",
    });
    cleanup();

    // A hoped-for day that's past, and no package named: neither is sent.
    one.mockResolvedValue(r("2020-01-31"));
    show(detail(1));
    fireEvent.click(await screen.findByRole("button", { name: "Ask them for a quote" }));
    expect(JSON.parse(decodeURIComponent(where().hash.slice("#fill=".length)))).toEqual({
      app_slug: "rheas-rooftops",
      skill: "pixel-art",
      request_md: "Rooftop tiles\n\nTwelve tiles, 32×32.",
    });
  });

  it("the maker's list shows 100 at a time, and the next 100 on request", async () => {
    signedIn();
    const first = Array.from({ length: 100 }, (_, i) => name(card(`bulk-${i + 1}`, `Bulk ${i + 1}`), "2026-11-10T11:00:00Z"));
    const r = req(1, { mine: true, owner: owner({ responses: first, responses_total: 101 }) });
    const one = vi.spyOn(api.people.requests, "one").mockImplementation(async (_id, q = {}) =>
      q.responses_offset ? { ...r, owner: { ...r.owner!, responses: [name(card("bulk-101", "Bulk 101"), "2026-11-10T12:00:00Z")] } } : r,
    );
    show(detail(1));
    const names = await screen.findByRole("region", { name: "Who put their name forward" });
    expect(within(names).getByText("Showing 100 of 101, in the order they put their name forward.")).toBeTruthy();
    fireEvent.click(within(names).getByRole("button", { name: "Show the next 100" }));
    const count = await within(names).findByText("Showing 101 of 101, in the order they put their name forward.", {}, { timeout: LONG });
    expect(one).toHaveBeenLastCalledWith(idOf(1), { responses_offset: 100 });
    expect(names.querySelectorAll("ol > li")).toHaveLength(101);
    expect(names.querySelector("ol > li:last-child .person-who b")?.textContent).toBe("Bulk 101");
    expect(within(names).queryByRole("button", { name: "Show the next 100" })).toBeNull();
    expect(count.getAttribute("role")).toBe("status");
    await waitFor(() => expect(document.activeElement).toBe(count));
  }, 30_000);

  it("each Show the next 100 reads the list again from the first name, so a name taken back meanwhile doesn't make one skip", async () => {
    signedIn();
    let server = Array.from({ length: 201 }, (_, i) => name(card(`bulk-${i + 1}`, `Bulk ${i + 1}`), "2026-11-10T11:00:00Z"));
    const page = (offset: number) => req(1, { mine: true, owner: owner({ responses: server.slice(offset, offset + 100), responses_total: server.length }) });
    const one = vi.spyOn(api.people.requests, "one").mockImplementation(async (_id, q = {}) => page(q.responses_offset ?? 0));
    show(detail(1));
    const names = await screen.findByRole("region", { name: "Who put their name forward" });
    expect(within(names).getByText("Showing 100 of 201, in the order they put their name forward.")).toBeTruthy();
    // The request takes the focus once it has loaded; the button is pressed after that.
    const title = screen.getByRole("heading", { level: 2, name: "Request 1" });
    await waitFor(() => expect(document.activeElement).toBe(title));
    const next = within(names).getByRole("button", { name: "Show the next 100" });
    next.focus();
    fireEvent.click(next);
    // Not disabled while it loads: the focus stays on it, and the count line says how many.
    expect(next.hasAttribute("disabled")).toBe(false);
    expect(await within(names).findByText("Showing 200 of 201, in the order they put their name forward.", {}, { timeout: LONG })).toBeTruthy();
    expect(document.activeElement).toBe(next);
    // Bulk 5 takes their name back: the places after it move up by one.
    server = server.filter((x) => x.person.handle !== "bulk-5");
    one.mockClear();
    fireEvent.click(next);
    const count = await within(names).findByText("Showing 200 of 200, in the order they put their name forward.", {}, { timeout: LONG });
    expect(one.mock.calls.map(([, q]) => q?.responses_offset)).toEqual([0, 100, 200]);
    // Read by places, Bulk 201 (now place 200) would have been skipped.
    const shown = [...names.querySelectorAll("ol > li .person-who b")].map((b) => b.textContent);
    expect(shown).toHaveLength(200);
    expect(shown).toContain("Bulk 201");
    expect(shown).not.toContain("Bulk 5");
    expect(new Set(shown).size).toBe(200);
    expect(within(names).queryByRole("button", { name: "Show the next 100" })).toBeNull();
    await waitFor(() => expect(document.activeElement).toBe(count));
  }, 30_000);

  it("the maker is told where a request stands when it's off the board", async () => {
    signedIn();
    const one = vi.spyOn(api.people.requests, "one");
    const cases: Array<[Partial<WorkRequestOwner>, RegExp]> = [
      [{ on_board: false }, /^Not showing on the board: its project isn't in the store right now\. It shows again when the project does, until Dec 10, 2026\.$/],
      [{ status: "expired", on_board: false, ended_at: "2026-12-10T09:00:00Z" }, /^This request ran out on Dec 10, 2026\.$/],
      [{ status: "hidden", on_board: false, hidden_reason: "Misleading pages" }, /^SLAPorSLOP hid this request: Misleading pages\. See the decision, where you can appeal\.$/],
    ];
    for (const [o, text] of cases) {
      one.mockResolvedValue(req(1, { mine: true, owner: owner(o) }));
      const { container } = show(detail(1));
      await screen.findByRole("region", { name: "Who put their name forward" });
      expect(container.querySelector(".request-state")?.textContent).toMatch(text);
      expect(screen.queryByRole("button", { name: "Close request" }) === null).toBe(o.status !== undefined);
      cleanup();
    }
  });
});

describe("the hire form filled in from an open request", () => {
  const VERA: CreativePage = {
    ...card("vera-pixels", "Vera Okafor", { skills: [{ slug: "pixel-art", name: "Pixel Art" }] }),
    about_md: "",
    ai_note: "",
    trader: false,
    turnaround: "",
    sections: ["services"],
    links: [],
    banner_url: null,
    design: null,
    works: [],
    services: [{ ...SPRITES, skill_name: "Pixel Art" }],
    reviews: [],
    worked_on: [],
    is_me: false,
    status: "live",
    mine: null,
    preview_set: null,
    hidden_reason: null,
    closed: false,
  };
  const FROM_REQUEST = { app_slug: "rheas-rooftops", skill: "pixel-art", request_md: "Rooftop tiles\n\nTwelve tiles.", service_id: "a1b2c3d4" };
  const fill = (fields: unknown) => `#fill=${encodeURIComponent(JSON.stringify(fields))}`;
  const showPerson = (state: unknown, hash = "") => {
    window.history.replaceState(null, "", `/people/vera-pixels${hash}`);
    return render(
      <MemoryRouter initialEntries={[{ pathname: "/people/vera-pixels", hash, state }]}>
        <Routes>
          <Route
            path="/people/:handle"
            element={
              <>
                <PersonPage />
                <Where />
              </>
            }
          />
        </Routes>
      </MemoryRouter>,
    );
  };
  beforeEach(() => {
    signedIn();
    vi.spyOn(api.people, "page").mockResolvedValue(VERA);
    vi.spyOn(api, "myApps").mockResolvedValue([{ ...ROOFTOPS, status: "listed" } as AppSummary]);
  });

  it("says the fill is from the request, only for the fill that came with that click", async () => {
    showPerson({ fromRequest: true }, fill(FROM_REQUEST));
    const notice = await screen.findByRole("status", { name: "Filled in from your open request" });
    expect(notice.textContent).toContain(
      "Filled in from your open request. Check everything, especially what you're asking for, word for word, then press Send request. Nothing is sent until you do.",
    );
    const form = screen.getByRole("form", { name: "Hire Vera Okafor" });
    // The notice shows with the fill; the values go in right after.
    await waitFor(() =>
      expect((within(form).getByRole("textbox", { name: /^What do you need, exactly\?/ }) as HTMLTextAreaElement).value).toBe("Rooftop tiles\n\nTwelve tiles."),
    );
    expect((within(form).getByRole("combobox", { name: "Package" }) as HTMLSelectElement).value).toBe("a1b2c3d4");
    expect((within(form).getByRole("checkbox", { name: /I understand: Vera Okafor will deliver/ }) as HTMLInputElement).checked).toBe(false);
    // The router state is gone once read: a reload is a plain visit.
    await waitFor(() => expect(where().state).toBeNull());

    // A second fill opened in the same tab gets the assistant's words.
    act(() => {
      window.history.replaceState(window.history.state, "", `/people/vera-pixels${fill({ request_md: "From my assistant." })}`);
      window.dispatchEvent(new HashChangeEvent("hashchange"));
    });
    expect(await screen.findByRole("status", { name: "Filled in by your AI assistant" })).toBeTruthy();
    expect(screen.queryByRole("status", { name: "Filled in from your open request" })).toBeNull();
  });

  it("after a reload the notice is the assistant's", async () => {
    // What a reload finds: no router state, and the fill kept in the tab if the form hadn't used it.
    sessionStorage.setItem("slop.handoff.hire", JSON.stringify({ at: Date.now(), fields: FROM_REQUEST, scope: "vera-pixels" }));
    showPerson(null);
    expect(await screen.findByRole("status", { name: "Filled in by your AI assistant" })).toBeTruthy();
    expect(screen.queryByRole("status", { name: "Filled in from your open request" })).toBeNull();
  });

  it("someone who can't be hired any more: the request's words say why it can't be sent", async () => {
    vi.mocked(api.people.page).mockResolvedValue({ ...VERA, can_hire: false, taking_work: false });
    showPerson({ fromRequest: true }, fill(FROM_REQUEST));
    expect(await screen.findByText("Your open request was ready to send to Vera Okafor, but they're fully booked, so it can't be sent.")).toBeTruthy();
  });
});

describe("HandoffNotice", () => {
  const fill: Fill<{ title: string }> = { values: { title: "Main theme" }, filled: ["title"], left: [], unknown: 0 };
  const props = { fill, labels: { title: "Title" }, check: "the title", button: "Post request", onClear: () => {}, clearLabel: "Clear the form" };

  it("names an open request as the source only when told; the assistant otherwise", () => {
    render(<HandoffNotice {...props} source="request" />);
    const from = screen.getByRole("status", { name: "Filled in from your open request" });
    expect(from.textContent).toContain("Filled in from your open request. Check everything, especially the title, then press Post request.");
    cleanup();
    render(<HandoffNotice {...props} />);
    const assistant = screen.getByRole("status", { name: "Filled in by your AI assistant" });
    expect(assistant.textContent).toContain("Filled in by your AI assistant. Check everything, especially the title, then press Post request.");
  });
});
