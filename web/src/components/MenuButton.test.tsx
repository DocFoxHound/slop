// SPDX-License-Identifier: AGPL-3.0-or-later
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MenuButton, type MenuSection } from "./MenuButton";

const setup = () => {
  const open = vi.fn();
  const remove = vi.fn();
  const beta = vi.fn();
  const sections: MenuSection[] = [
    { items: [{ kind: "link", label: "Open its record", to: "/admin?tab=apps&app=x" }, { label: "Open later", onSelect: open }] },
    {
      label: "Stage",
      items: [
        { kind: "radio", label: "Beta", checked: false, onSelect: beta },
        { kind: "radio", label: "Released", checked: true, onSelect: vi.fn() },
      ],
    },
    { items: [{ label: "Remove", danger: true, disabled: "Can't remove it yet.", onSelect: remove }] },
  ];
  render(
    <MemoryRouter>
      <button>before</button>
      <MenuButton label="Actions for Moss" sections={sections} />
    </MemoryRouter>,
  );
  return { button: screen.getByRole("button", { name: "Actions for Moss" }), open, remove, beta };
};

describe("MenuButton", () => {
  afterEach(cleanup);

  it("is a menu button, closed until opened", () => {
    const { button } = setup();
    expect(button.getAttribute("aria-haspopup")).toBe("menu");
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("opens on the first item, moves with the arrows, Home, End and a letter, and Escape goes back", () => {
    const { button } = setup();
    fireEvent.click(button);
    expect(button.getAttribute("aria-expanded")).toBe("true");
    const menu = screen.getByRole("menu", { name: "Actions for Moss" });
    expect(button.getAttribute("aria-controls")).toBe(menu.id);
    const record = screen.getByRole("menuitem", { name: "Open its record" });
    expect(document.activeElement).toBe(record);
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Open later" }));
    fireEvent.keyDown(menu, { key: "End" });
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Remove" }));
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(record);
    fireEvent.keyDown(menu, { key: "ArrowUp" });
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Remove" }));
    fireEvent.keyDown(menu, { key: "Home" });
    expect(document.activeElement).toBe(record);
    fireEvent.keyDown(menu, { key: "b" });
    expect(document.activeElement).toBe(screen.getByRole("menuitemradio", { name: "Beta" }));
    fireEvent.keyDown(menu, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(button);
  });

  it("opens on the last item with the up arrow", () => {
    const { button } = setup();
    fireEvent.keyDown(button, { key: "ArrowUp" });
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Remove" }));
  });

  it("chooses an item, says which stage is on, and keeps a disabled item's reason", () => {
    const { button, open, remove, beta } = setup();
    fireEvent.click(button);
    expect(screen.getByRole("group", { name: "Stage" })).toBeTruthy();
    expect(screen.getByRole("menuitemradio", { name: "Released" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("menuitemradio", { name: "Beta" }).getAttribute("aria-checked")).toBe("false");
    const disabled = screen.getByRole("menuitem", { name: "Remove" });
    expect(disabled.getAttribute("aria-disabled")).toBe("true");
    expect(document.getElementById(disabled.getAttribute("aria-describedby")!)?.textContent).toBe("Can't remove it yet.");
    fireEvent.click(disabled);
    expect(remove).not.toHaveBeenCalled();
    expect(screen.getByRole("menu")).toBeTruthy();
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Beta" }));
    expect(beta).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(button);
    fireEvent.click(button);
    fireEvent.click(screen.getByRole("menuitem", { name: "Open later" }));
    expect(open).toHaveBeenCalledOnce();
  });

  it("closes on Tab and on a click outside", () => {
    const { button } = setup();
    fireEvent.click(button);
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Tab" });
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.click(button);
    fireEvent.mouseDown(screen.getByRole("button", { name: "before" }));
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
