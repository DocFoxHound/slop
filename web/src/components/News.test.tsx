// SPDX-License-Identifier: AGPL-3.0-or-later
// Maker news (News.tsx): the text through the chat renderer only, the app page's section, the
// person's switch, and the maker's panel on Manage.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, ApiError, type AppDetail, type NewsPage, type NewsPost } from "../api";
import { openExternal } from "../platform";
import { AppNews, NewsBody, NewsPanel, NewsSwitch } from "./News";

vi.mock("../session", () => ({ useSession: () => ({ me: null }) }));
vi.mock("../platform", async (original) => ({ ...(await original<typeof import("../platform")>()), openExternal: vi.fn(async () => {}) }));

const post = (n: number, over: Partial<NewsPost> = {}): NewsPost => ({
  id: `00000000-0000-7000-8000-${String(n).padStart(12, "0")}`,
  headline: `Headline ${n}`,
  body: `Text ${n}`,
  created_at: `2026-11-${String(20 - n).padStart(2, "0")}T12:00:00Z`,
  edited: false,
  hidden: false,
  ...over,
});
const page = (news: NewsPost[], more = false, posting?: NewsPage["posting"]): NewsPage => ({ news, more, ...(posting ? { posting } : {}) });
const app = (over: Partial<AppDetail> = {}) => ({ slug: "pong", title: "Pong", stage: "beta", is_mine: false, ...over }) as AppDetail;
const section = () => screen.getByRole("region", { name: "News" });

// jsdom has no modal dialogs; the Report button's dialog only needs to open and close.
HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
  this.open = true;
};
HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
  this.open = false;
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("NewsBody", () => {
  it("renders simple Markdown and links that leave SLAPorSLOP; never raw HTML, pictures or headings", () => {
    const { container } = render(
      <NewsBody body={"**Five** new levels. [Notes](https://example.com/notes)\n\n<script>alert(1)</script>\n\n![cat](https://example.com/cat.png)\n\n# Big"} />,
    );
    expect(container.querySelector("strong")?.textContent).toBe("Five");
    const link = screen.getByRole("link", { name: "Notes" });
    expect(link.getAttribute("href")).toBe("https://example.com/notes");
    expect(link.hasAttribute("data-external")).toBe(true);
    expect(link.getAttribute("rel")).toContain("nofollow");
    expect(container.querySelector("script")).toBeNull();
    expect(container.textContent).toContain("<script>alert(1)</script>");
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("h1, h2, h3, h4, h5, h6")).toBeNull();
    fireEvent.click(link);
    expect(openExternal).toHaveBeenCalledWith("https://example.com/notes");
  });
});

describe("AppNews", () => {
  const show = (props: Partial<Parameters<typeof AppNews>[0]> = {}, path = "/app/pong") =>
    render(
      <MemoryRouter initialEntries={[path]}>
        <AppNews app={app()} marked={false} newsOn={true} onNewsOn={() => {}} {...props} />
      </MemoryRouter>,
    );

  it("shows nothing with no posts and no mark, while it loads, or when it can't load", async () => {
    let answer: (p: NewsPage) => void = () => {};
    const list = vi.spyOn(api.news, "list").mockReturnValue(new Promise<NewsPage>((resolve) => (answer = resolve)));
    const { container } = show();
    await waitFor(() => expect(list).toHaveBeenCalledWith("pong", { limit: 3 }));
    expect(container.innerHTML).toBe("");
    await act(async () => answer(page([])));
    expect(container.innerHTML).toBe("");
    cleanup();
    vi.spyOn(api.news, "list").mockRejectedValue(new ApiError(500, "internal", "Something broke."));
    const failed = show({ marked: true, app: app({ stage: "released" }) });
    await act(async () => {});
    expect(failed.container.innerHTML).toBe("");
  });

  it("keeps the section for a released app's switch, with no posts yet", async () => {
    vi.spyOn(api.news, "list").mockResolvedValue(page([]));
    show({ marked: true, app: app({ stage: "released" }) });
    const box = (await screen.findByRole("checkbox", { name: "Tell me about news from this app" })) as HTMLInputElement;
    expect(box.checked).toBe(true);
    expect(within(section()).getByText("No news from the maker yet.")).toBeTruthy();
    expect(within(section()).queryByRole("listitem")).toBeNull();
  });

  it("shows the newest three, each with Report; Show older news adds ten, moves the focus and goes when there are no more", async () => {
    const list = vi
      .spyOn(api.news, "list")
      .mockResolvedValueOnce(page([post(1, { edited: true }), post(2), post(3)], true))
      .mockResolvedValueOnce(page(Array.from({ length: 10 }, (_, i) => post(4 + i))));
    show();
    expect(await screen.findByRole("heading", { name: "Headline 1", level: 3 })).toBeTruthy();
    expect(within(section()).getAllByRole("listitem")).toHaveLength(3);
    expect(within(section()).getByText(/· edited/)).toBeTruthy();
    expect(within(section()).getAllByRole("button", { name: "Report" })).toHaveLength(3);
    expect(screen.queryByRole("checkbox")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show older news" }));
    await waitFor(() => expect(within(section()).getAllByRole("listitem")).toHaveLength(13));
    expect(list).toHaveBeenLastCalledWith("pong", { before: post(3).id, limit: 10 });
    await waitFor(() => expect(document.activeElement?.textContent).toBe("Headline 4"));
    expect(screen.queryByRole("button", { name: "Show older news" })).toBeNull();
    // Each post has its own address on the page (the staff link: /app/pong#news-<id>).
    expect(document.getElementById(`news-${post(4).id}`)).toBeTruthy();
  });

  it("a failed Show older news says why and keeps the three", async () => {
    vi.spyOn(api.news, "list")
      .mockResolvedValueOnce(page([post(1), post(2), post(3)], true))
      .mockRejectedValueOnce(new ApiError(500, "internal", "Something broke on SLAPorSLOP's side."));
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Show older news" }));
    expect((await within(section()).findByRole("alert")).textContent).toBe("Something broke on SLAPorSLOP's side.");
    expect(within(section()).getAllByRole("listitem")).toHaveLength(3);
    expect(screen.getByRole("button", { name: "Show older news" })).toBeTruthy();
  });

  it("the maker sees what the public sees: no hidden post, no Report", async () => {
    vi.spyOn(api.news, "list").mockResolvedValue(
      page([post(1), post(2, { hidden: true, hidden_reason: "Spam" })], false, { allowed: true, why_not: null, next_post_at: null }),
    );
    show({ app: app({ is_mine: true }) });
    await screen.findByRole("heading", { name: "Headline 1" });
    expect(screen.queryByText("Headline 2")).toBeNull();
    expect(screen.queryByRole("button", { name: "Report" })).toBeNull();
  });

  it("a link to one post asks for more, so the post is there to scroll to", async () => {
    const list = vi.spyOn(api.news, "list").mockResolvedValue(page([post(1)]));
    show({}, `/app/pong#news-${post(1).id}`);
    await screen.findByRole("heading", { name: "Headline 1" });
    expect(list).toHaveBeenCalledWith("pong", { limit: 50 });
  });

  it("with a mark, the switch is in the section too", async () => {
    vi.spyOn(api.news, "list").mockResolvedValue(page([post(1)]));
    show({ marked: true, newsOn: false });
    const box = (await screen.findByRole("checkbox", { name: "Tell me about news from this app" })) as HTMLInputElement;
    expect(box.checked).toBe(false);
  });
});

describe("NewsSwitch", () => {
  it("switches news for the app, and on failure says why and stays as it was", async () => {
    const setNews = vi.spyOn(api.interested, "setNews").mockResolvedValue(undefined);
    const changed = vi.fn();
    render(<NewsSwitch slug="pong" on={true} onChange={changed} />);
    const box = screen.getByRole("checkbox", { name: "Tell me about news from this app" }) as HTMLInputElement;
    box.focus();
    fireEvent.click(box);
    await waitFor(() => expect(changed).toHaveBeenCalledWith(false));
    expect(setNews).toHaveBeenCalledWith("pong", false);
    await waitFor(() => expect(box.disabled).toBe(false));
    expect(document.activeElement).toBe(box);
    expect(screen.queryByRole("alert")).toBeNull();

    setNews.mockRejectedValueOnce(new ApiError(429, "rate_limited", "too many Interested marks for now: try again in a while"));
    changed.mockClear();
    fireEvent.click(box);
    expect((await screen.findByRole("alert")).textContent).toBe("too many Interested marks for now: try again in a while");
    expect(changed).not.toHaveBeenCalled();
    expect(box.checked).toBe(true);
  });
});

describe("NewsPanel", () => {
  const show = () =>
    render(
      <MemoryRouter>
        <NewsPanel slug="pong" />
      </MemoryRouter>,
    );
  const panel = () => screen.getByRole("region", { name: "News" });
  const allowed = { allowed: true, why_not: null, next_post_at: null } as const;

  it("says by the posting state: Write news, when the next post is allowed, or that the page isn't in the store", async () => {
    vi.spyOn(api.news, "list").mockResolvedValue(page([], false, allowed));
    show();
    expect(await screen.findByRole("button", { name: "Write news" })).toBeTruthy();
    expect(within(panel()).getByText(/One new post per app per day\./)).toBeTruthy();
    expect(within(panel()).getByText(/are told under their bell, unless they switched news off for it\./)).toBeTruthy();
    cleanup();

    vi.spyOn(api.news, "list").mockResolvedValue(page([post(1)], false, { allowed: false, why_not: "wait", next_post_at: "2026-11-12T16:08:00Z" }));
    show();
    expect(await screen.findByText(/^You can post again on /)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Write news" })).toBeNull();
    expect(screen.getByRole("button", { name: "Edit: Headline 1" })).toBeTruthy();
    cleanup();

    vi.spyOn(api.news, "list").mockResolvedValue(page([post(1)], false, { allowed: false, why_not: "not_in_store", next_post_at: null }));
    show();
    expect(await screen.findByText(/isn't in the store now, so news can't be posted or changed\. You can still remove a post\./)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Edit/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Remove: Headline 1" })).toBeTruthy();
    cleanup();

    // Not in the store and nothing posted: nothing to show.
    const list = vi.spyOn(api.news, "list").mockResolvedValue(page([], false, { allowed: false, why_not: "not_in_store", next_post_at: null }));
    const { container } = show();
    await waitFor(() => expect(list).toHaveBeenCalledWith("pong", { limit: 50 }));
    await act(async () => {});
    expect(container.innerHTML).toBe("");
  });

  it("a hidden post says why, links to the appeal while one can be made, and has no buttons", async () => {
    vi.spyOn(api.news, "list").mockResolvedValue(
      page(
        [
          post(1, { hidden: true, hidden_reason: "Misleading pages", can_appeal: true }),
          post(2, { hidden: true, hidden_reason: "Spam", can_appeal: false }),
        ],
        false,
        allowed,
      ),
    );
    show();
    const row = (await screen.findByText("Headline 1")).closest("li")!;
    expect(within(row).getByText("HIDDEN")).toBeTruthy();
    expect(row.textContent).toContain("Removed by a moderation decision: Misleading pages. You can appeal it from your account.");
    expect(within(row).getByRole("link", { name: "your account" }).getAttribute("href")).toBe("/account#decisions");
    expect(within(row).queryByRole("button")).toBeNull();
    // Appealed already, or six months on: no appeal is promised.
    const decided = screen.getByText("Headline 2").closest("li")!;
    expect(decided.textContent).toContain("Removed by a moderation decision: Spam. See the decision on your account.");
    expect(decided.textContent).not.toContain("appeal");
    expect(within(decided).getByRole("link", { name: "your account" }).getAttribute("href")).toBe("/account#decisions");
  });

  it("writes a post: the focus goes to the headline, a refused field is marked, Cancel goes back", async () => {
    vi.spyOn(api.news, "list").mockResolvedValue(page([], false, allowed));
    const send = vi
      .spyOn(api.news, "post")
      .mockRejectedValueOnce(new ApiError(422, "invalid", "1 to 100 characters, on one line", "headline"))
      .mockResolvedValueOnce(post(1));
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Write news" }));
    const headline = screen.getByRole("textbox", { name: "Headline" }) as HTMLInputElement;
    expect(document.activeElement).toBe(headline);
    expect(headline.maxLength).toBe(100);
    expect(document.getElementById(headline.getAttribute("aria-describedby")!)?.textContent).toContain("It's what people see in their notifications.");
    const text = screen.getByRole("textbox", { name: "News" }) as HTMLTextAreaElement;
    expect(text.maxLength).toBe(2000);
    expect(screen.getByText(/Posting tells the people who marked this app Interested, unless they switched news off/)).toBeTruthy();
    fireEvent.change(headline, { target: { value: "Beta opens on Friday" } });
    fireEvent.change(text, { target: { value: "The first three areas." } });
    const postButton = screen.getByRole("button", { name: "Post news" });
    postButton.focus();
    fireEvent.click(postButton);
    // Said next to the field, which says it too and takes the focus to be fixed.
    const said = await screen.findByRole("alert");
    expect(said.textContent).toBe("1 to 100 characters, on one line");
    expect(said.classList.contains("field-error")).toBe(true);
    expect(headline.getAttribute("aria-invalid")).toBe("true");
    expect(headline.getAttribute("aria-describedby")!.split(" ")).toContain(said.id);
    expect(text.hasAttribute("aria-invalid")).toBe(false);
    expect(text.getAttribute("aria-describedby")).toBe("news-body-hint");
    await waitFor(() => expect(document.activeElement).toBe(headline));

    fireEvent.click(screen.getByRole("button", { name: "Post news" }));
    await waitFor(() => expect(send).toHaveBeenCalledTimes(2));
    expect(send).toHaveBeenLastCalledWith("pong", { headline: "Beta opens on Friday", body: "The first three areas." });
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "Headline" })).toBeNull());
    expect(document.activeElement).toBe(screen.getByRole("heading", { name: "News", level: 2 }));
    expect(screen.getByRole("status").textContent).toBe("Posted.");

    fireEvent.click(screen.getByRole("button", { name: "Write news" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Write news" }));
  });

  it("edits a post in its row, and the focus goes back to Edit", async () => {
    vi.spyOn(api.news, "list").mockResolvedValue(page([post(1)], false, { allowed: false, why_not: "wait", next_post_at: "2026-11-12T16:08:00Z" }));
    const edit = vi.spyOn(api.news, "edit").mockResolvedValue(post(1, { headline: "Fixed" }));
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Edit: Headline 1" }));
    const headline = screen.getByRole("textbox", { name: "Headline" }) as HTMLInputElement;
    expect(headline.value).toBe("Headline 1");
    expect((screen.getByRole("textbox", { name: "News" }) as HTMLTextAreaElement).value).toBe("Text 1");
    // Nobody is told again, so no warning about telling them.
    expect(screen.queryByText(/Posting tells/)).toBeNull();
    fireEvent.change(headline, { target: { value: "Fixed" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(edit).toHaveBeenCalledWith("pong", post(1).id, { headline: "Fixed", body: "Text 1" }));
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("button", { name: "Edit: Headline 1" })));
  });

  it("removes a post only after asking, and Keep it changes nothing", async () => {
    const list = vi.spyOn(api.news, "list").mockResolvedValue(page([post(1), post(2)], false, allowed));
    const remove = vi.spyOn(api.news, "remove").mockResolvedValue(undefined);
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Remove: Headline 1" }));
    expect(screen.getByText("Remove this post? It doesn't give the day's post back.")).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Keep it" }));
    fireEvent.click(screen.getByRole("button", { name: "Keep it" }));
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Remove: Headline 1" }));
    expect(remove).not.toHaveBeenCalled();

    // The question is the name of the group its two buttons are in.
    fireEvent.click(screen.getByRole("button", { name: "Remove: Headline 1" }));
    const asked = screen.getByRole("group", { name: "Remove this post? It doesn't give the day's post back." });
    expect(within(asked).getByRole("button", { name: "Keep it" })).toBe(document.activeElement);

    // Refused: it says why, and the focus is back on Remove post. (A browser takes the focus off
    // the button while it's disabled; jsdom doesn't, so the request does it.)
    remove.mockImplementationOnce(async () => {
      (document.activeElement as HTMLElement).blur();
      throw new ApiError(500, "internal", "Something broke on SLAPorSLOP's side.");
    });
    const confirm = within(asked).getByRole("button", { name: "Remove post" });
    confirm.focus();
    fireEvent.click(confirm);
    expect((await screen.findByRole("alert")).textContent).toBe("Something broke on SLAPorSLOP's side.");
    await waitFor(() => expect(document.activeElement).toBe(confirm));

    list.mockResolvedValue(page([post(2)], false, allowed));
    fireEvent.click(confirm);
    await waitFor(() => expect(remove).toHaveBeenCalledTimes(2));
    expect(remove).toHaveBeenLastCalledWith("pong", post(1).id);
    await waitFor(() => expect(screen.queryByText("Headline 1")).toBeNull());
    expect(screen.getByText("Headline 2")).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole("heading", { name: "News", level: 2 }));
    expect(screen.getByRole("status").textContent).toBe("Removed.");
  });

  it("a refusal with no field shows at the end, and the focus goes back to the button", async () => {
    vi.spyOn(api.news, "list").mockResolvedValue(page([], false, allowed));
    // A browser takes the focus off the button while it's disabled; jsdom doesn't, so the request does it.
    vi.spyOn(api.news, "post").mockImplementation(async () => {
      (document.activeElement as HTMLElement).blur();
      throw new ApiError(409, "conflict", "news can be posted or changed only while the app's page is in the store");
    });
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Write news" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Headline" }), { target: { value: "Beta opens on Friday" } });
    fireEvent.change(screen.getByRole("textbox", { name: "News" }), { target: { value: "The first three areas." } });
    const button = screen.getByRole("button", { name: "Post news" });
    button.focus();
    fireEvent.click(button);
    const said = await screen.findByRole("alert");
    expect(said.textContent).toBe("news can be posted or changed only while the app's page is in the store");
    expect(said.classList.contains("form-error")).toBe(true);
    expect(screen.getByRole("textbox", { name: "Headline" }).hasAttribute("aria-invalid")).toBe(false);
    await waitFor(() => expect(document.activeElement).toBe(button));
  });

  it("reaches posts older than the first fifty: Show older posts adds the next page and moves the focus to it", async () => {
    const fifty = Array.from({ length: 50 }, (_, i) => post(1 + i, { created_at: "2026-10-01T12:00:00Z" }));
    const list = vi
      .spyOn(api.news, "list")
      .mockImplementation(async (_slug, o = {}) =>
        o.before ? page([post(51, { hidden: true, hidden_reason: "Spam", can_appeal: true }), post(52)]) : page(fifty, true, allowed),
      );
    show();
    expect(await screen.findByText("Headline 50")).toBeTruthy();
    expect(screen.queryByText("Headline 51")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show older posts" }));
    expect(await screen.findByText("Headline 52")).toBeTruthy();
    expect(list).toHaveBeenLastCalledWith("pong", { before: post(50).id, limit: 50 });
    // The hidden one beyond the fifty keeps its line and the way to appeal.
    expect(screen.getByText("Headline 51").closest("li")!.textContent).toContain("You can appeal it from your account.");
    await waitFor(() => expect(document.activeElement?.textContent).toBe("Headline 51"));
    expect(screen.queryByRole("button", { name: "Show older posts" })).toBeNull();
    expect(screen.getByRole("button", { name: "Edit: Headline 52" })).toBeTruthy();
  });
});
