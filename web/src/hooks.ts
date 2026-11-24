// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from "react";

import { ApiError } from "./api";
import { siteName } from "./browsing";

export interface Loaded<T> {
  data: T | undefined;
  error: ApiError | undefined;
  loading: boolean;
  reload: () => void;
}

/**
 * Runs `load` whenever `deps` change and keeps the latest result. Responses from earlier,
 * superseded loads are ignored.
 */
export function useLoad<T>(load: () => Promise<T>, deps: unknown[]): Loaded<T> {
  const [state, setState] = useState<{ data?: T; error?: ApiError; loading: boolean }>({ loading: true });
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    let current = true;
    setState((s) => ({ ...s, loading: true }));
    load().then(
      (data) => current && setState({ data, loading: false }),
      (err: unknown) =>
        current &&
        setState({
          loading: false,
          error: err instanceof ApiError ? err : new ApiError(0, "unknown", String(err)),
        }),
    );
    return () => {
      current = false;
    };
  }, [...deps, nonce]);

  const reload = useCallback(() => setNonce((n) => n + 1), []);
  return { data: state.data, error: state.error, loading: state.loading, reload };
}

export function useDocumentTitle(title: string | undefined) {
  useEffect(() => {
    document.title = title ? `${title} · ${siteName()}` : siteName();
  }, [title]);
}

/**
 * Calls `tick` every `ms` milliseconds while `ms` is set (null stops it), skipping ticks while the
 * page is hidden. For pages that follow something under way, like a release being checked.
 */
export function useInterval(tick: () => void, ms: number | null) {
  useEffect(() => {
    if (ms === null) return;
    const timer = window.setInterval(() => {
      if (typeof document !== "undefined" && document.hidden) return;
      tick();
    }, ms);
    return () => window.clearInterval(timer);
  }, [tick, ms]);
}

/** The time now, updated every `ms` milliseconds while `ms` is set, for "started 12 minutes ago". */
export function useNow(ms: number | null): number {
  const [now, setNow] = useState(() => Date.now());
  const tick = useCallback(() => setNow(Date.now()), []);
  useInterval(tick, ms);
  return now;
}

/**
 * A switch that saves asynchronously: shows the choice at once and keeps showing it until the
 * saved value catches up (or the save fails: `show(undefined)`), instead of snapping back to the
 * old value while the save and the reload are on their way.
 */
export function useChosen<T>(saved: T): [T, (choice: T | undefined) => void] {
  const [chosen, setChosen] = useState<T>();
  useEffect(() => {
    if (chosen !== undefined && chosen === saved) setChosen(undefined);
  }, [saved, chosen]);
  return [chosen ?? saved, setChosen];
}

/** Whether a CSS media query matches, kept up to date as the window changes. */
export function useMediaQuery(query: string): boolean {
  const get = () => typeof window !== "undefined" && Boolean(window.matchMedia?.(query).matches);
  const [matches, setMatches] = useState(get);
  useEffect(() => {
    const list = window.matchMedia?.(query);
    if (!list) return;
    const changed = () => setMatches(list.matches);
    changed();
    list.addEventListener("change", changed);
    return () => list.removeEventListener("change", changed);
  }, [query]);
  return matches;
}

/**
 * A confirm step that replaces the button that opened it: once it closes with "Not yet", focus goes
 * back to that button (so keyboard and screen reader users aren't dropped at the top of the page).
 * Returns the opening button's ref, and `close` for the step's own way out.
 */
export function useConfirmStep(open: boolean, setOpen: (open: boolean) => void) {
  const opener = useRef<HTMLButtonElement>(null);
  const back = useRef(false);
  useEffect(() => {
    if (!open && back.current) {
      back.current = false;
      opener.current?.focus();
    }
  }, [open]);
  const close = useCallback(() => {
    back.current = true;
    setOpen(false);
  }, [setOpen]);
  return { opener, close };
}
