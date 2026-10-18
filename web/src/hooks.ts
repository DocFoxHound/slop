import { useCallback, useEffect, useState } from "react";

import { ApiError } from "./api";

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
    document.title = title ? `${title} · Slop` : "Slop";
  }, [title]);
}
