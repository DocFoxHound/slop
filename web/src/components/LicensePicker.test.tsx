// SPDX-License-Identifier: AGPL-3.0-or-later
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { api } from "../api";
import { LicensePicker } from "./LicensePicker";

function Harness({ repo, initial = "" }: { repo: string; initial?: string }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <LicensePicker repoUrl={repo} value={value} onChange={setValue} />
      <output data-testid="value">{value}</output>
    </>
  );
}

const detected = (spdx: string | null, name: string | null = null) =>
  vi.spyOn(api, "detectLicense").mockResolvedValue({ spdx, name, note: spdx ? null : "no licence file found" });

async function settle() {
  await act(async () => {
    vi.advanceTimersByTime(700);
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("LicensePicker", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("fills in the licence it detects", async () => {
    const spy = detected("Apache-2.0", "Apache License 2.0");
    render(<Harness repo="https://github.com/octo/game" />);
    await settle();
    expect(spy).toHaveBeenCalledWith("https://github.com/octo/game");
    expect(screen.getByTestId("value").textContent).toBe("Apache-2.0");
    expect(screen.getByText(/Detected from your repository: Apache License 2.0/)).toBeTruthy();
  });

  it("clears a detected licence when the repository changes to one without", async () => {
    const spy = detected("MIT", "MIT License");
    const view = render(<Harness repo="https://github.com/octo/game" />);
    await settle();
    expect(screen.getByTestId("value").textContent).toBe("MIT");
    spy.mockResolvedValue({ spdx: null, name: null, note: "no licence file found" });
    view.rerender(<Harness repo="https://github.com/octo/other" />);
    await settle();
    expect(screen.getByTestId("value").textContent).toBe("");
  });

  it("never overrides a licence the maker chose", async () => {
    detected("MIT", "MIT License");
    render(<Harness repo="https://github.com/octo/game" initial="GPL-3.0-only" />);
    await settle();
    expect(screen.getByTestId("value").textContent).toBe("GPL-3.0-only");
  });

  it("asks rather than guesses when GitHub can't tell 'only' from 'or later'", async () => {
    detected("GPL-3.0", "GNU General Public License v3.0");
    render(<Harness repo="https://github.com/octo/game" />);
    await settle();
    expect(screen.getByTestId("value").textContent).toBe("");
    expect(screen.getByText(/Choose "only" or "or later"/)).toBeTruthy();
  });

  it("offers Other for any SPDX ID", async () => {
    detected(null);
    render(<Harness repo="" />);
    fireEvent.change(screen.getByLabelText("Licence"), { target: { value: "__other" } });
    const box = screen.getByLabelText("Other licence (SPDX ID)");
    fireEvent.change(box, { target: { value: "CDDL-1.0" } });
    expect(screen.getByTestId("value").textContent).toBe("CDDL-1.0");
  });

  it("doesn't call GitHub until there's a repository URL", async () => {
    const spy = detected("MIT");
    render(<Harness repo="https://github.com" />);
    await settle();
    expect(spy).not.toHaveBeenCalled();
  });
});
