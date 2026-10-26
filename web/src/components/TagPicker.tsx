// SPDX-License-Identifier: AGPL-3.0-or-later
// A store page's tags (Publish → New app and Edit store page): grouped by category, with a search
// across all of them, since there are over a hundred (migration 0089). Groups that rarely fit the
// app's type start closed; each group's summary names what's picked in it, so nothing picked is
// out of sight. Warns, without refusing, when a picked tag's category won't list the app (Kids &
// Family on a 12+ app; catalog.rs `push_category_limits`).

import { useState, type KeyboardEvent } from "react";

import type { AppKind, Category, ContentLabel, TagRef } from "../api";
import { CONTENT_LABEL } from "../format";

/** The API's limit (catalog.rs MAX_TAGS). */
export const MAX_TAGS = 10;

/** Categories that start closed for each type: games rarely need tool tags, and tools game ones. */
const CLOSED: Record<AppKind, string[]> = { game: ["tools", "creative", "utilities"], app: ["games"] };

/** A tag matches a search by its name or address, ignoring case ("open w" finds Open World). */
export function tagMatches(tag: TagRef, query: string): boolean {
  const q = query.trim().toLowerCase();
  return !q || tag.name.toLowerCase().includes(q) || tag.slug.replace(/-/g, " ").includes(q) || tag.slug.includes(q);
}

/** Picked tags of a category that won't list the app, and why. */
export interface Mismatch {
  category: Category;
  tags: TagRef[];
  /** Rated above the category's limit. */
  tooOld: boolean;
  /** Content labels the category leaves out. */
  labels: ContentLabel[];
}

/** The picked tags whose category won't list an app with this rating and these labels. */
export function categoryMismatches(categories: Category[], tags: string[], minAge: number, labels: ContentLabel[]): Mismatch[] {
  return categories.flatMap((category) => {
    const picked = category.tags.filter((t) => tags.includes(t.slug));
    if (picked.length === 0) return [];
    const tooOld = category.max_age != null && minAge > category.max_age;
    const clashing = labels.filter((l) => category.excluded_labels?.includes(l));
    return tooOld || clashing.length > 0 ? [{ category, tags: picked, tooOld, labels: clashing }] : [];
  });
}

/** "horror, strong language and gambling themes" (or "or …"). */
function list(words: string[], last = "and"): string {
  return words.length < 2 ? (words[0] ?? "") : `${words.slice(0, -1).join(", ")} ${last} ${words[words.length - 1]}`;
}

const labelWords = (labels: ContentLabel[]) => labels.map((l) => CONTENT_LABEL[l].toLowerCase());

export function TagPicker({
  categories,
  value,
  onChange,
  kind,
  minAge,
  labels,
  error,
}: {
  categories: Category[];
  value: string[];
  onChange: (tags: string[]) => void;
  kind: AppKind;
  minAge: number;
  labels: ContentLabel[];
  error?: string;
}) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(categories.map((c) => [c.slug, !CLOSED[kind].includes(c.slug) || c.tags.some((t) => value.includes(t.slug))])),
  );
  const searching = query.trim() !== "";
  const full = value.length >= MAX_TAGS;
  const toggle = (slug: string) => {
    if (value.includes(slug)) return onChange(value.filter((t) => t !== slug));
    onChange([...value, slug]);
    // A tag picked from a search stays in sight once the search is cleared.
    const home = categories.find((c) => c.tags.some((t) => t.slug === slug));
    if (home) setOpen((o) => ({ ...o, [home.slug]: true }));
  };
  const groups = categories
    .map((c) => ({ category: c, tags: c.tags.filter((t) => tagMatches(t, query)) }))
    .filter((g) => g.tags.length > 0);
  const found = groups.flatMap((g) => g.tags);
  const mismatches = categoryMismatches(categories, value, minAge, labels);

  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    // Enter never sends the form from here; with one tag left, it picks (or unpicks) it.
    if (e.key === "Enter") {
      e.preventDefault();
      const only = found.length === 1 ? found[0].slug : undefined;
      if (only && (value.includes(only) || !full)) toggle(only);
    } else if (e.key === "Escape" && searching) {
      e.preventDefault();
      setQuery("");
    }
  };

  return (
    <fieldset className="tag-picker">
      <legend>
        Tags <span className="muted">({value.length}/{MAX_TAGS})</span>
      </legend>
      <input
        type="search"
        className="tag-search"
        aria-label="Find a tag"
        placeholder="Find a tag…"
        value={query}
        maxLength={40}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={onKey}
      />
      <p className={searching && found.length === 0 ? "muted small" : "sr-only"} role="status">
        {searching ? (found.length === 0 ? `No tag matches “${query.trim()}”.` : `${found.length} ${found.length === 1 ? "tag matches" : "tags match"}.`) : ""}
      </p>
      {groups.map(({ category: c, tags }) => {
        const picked = c.tags.filter((t) => value.includes(t.slug));
        return (
          <details
            key={c.slug}
            className="tag-group"
            open={searching || open[c.slug]}
            onToggle={(e) => {
              const now = e.currentTarget.open;
              if (!searching) setOpen((o) => (o[c.slug] === now ? o : { ...o, [c.slug]: now }));
            }}
          >
            <summary>
              <span className="tag-group-name">{c.name}</span>
              {picked.length > 0 && <span className="muted"> · {picked.map((t) => t.name).join(", ")}</span>}
            </summary>
            <div className="chips">
              {tags.map((t) => {
                const on = value.includes(t.slug);
                return (
                  <button
                    type="button"
                    key={t.slug}
                    className={`chip ${on ? "on" : ""}`}
                    aria-pressed={on}
                    disabled={!on && full}
                    onClick={() => toggle(t.slug)}
                  >
                    {t.name}
                  </button>
                );
              })}
            </div>
          </details>
        );
      })}
      <div role="status">
        {mismatches.map((m) => (
          <p key={m.category.slug} className="tag-warning small">
            ⚠️ {m.category.name} lists only apps rated {m.category.max_age ?? 18}+ or under
            {m.category.excluded_labels?.length ? `, without ${list(labelWords(m.category.excluded_labels), "or")}` : ""}.{" "}
            {m.tooOld ? `This one is rated ${minAge}+` : `This one is labelled ${list(labelWords(m.labels))}`}, so{" "}
            {list(m.tags.map((t) => t.name))} won't show it there. You can still save: check the age rating and labels,
            or remove {m.tags.length === 1 ? "the tag" : "those tags"}.
          </p>
        ))}
      </div>
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
    </fieldset>
  );
}
