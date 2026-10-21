// SPDX-License-Identifier: AGPL-3.0-or-later
// The in-game overlay (crates/slop-desktop/src/overlay.rs): Shift+Tab over a fullscreen game
// started from Slop. A small panel with chats and friends over a dimmed game; Esc, Shift+Tab or a
// click outside goes back to the game. Kept deliberately small.

import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { MemoryRouter, NavLink, Route, Routes, useNavigate } from "react-router";

import { ChatApp } from "./pages/Chat";
import { FriendsPage } from "./pages/Friends";
import { desktop } from "./platform";
import { SessionProvider } from "./session";
import "./styles.css";

function Overlay() {
  const [game, setGame] = useState<string | null>(null);
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void desktop.onOverlayShown((e) => setGame(e.game)).then((u) => (unlisten = u));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector("dialog[open]")) void desktop.overlayHide();
    };
    window.addEventListener("keydown", onKey);
    const clock = window.setInterval(() => setNow(new Date()), 15000);
    return () => {
      unlisten?.();
      window.removeEventListener("keydown", onKey);
      window.clearInterval(clock);
    };
  }, []);
  return (
    <div
      className="overlay-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) void desktop.overlayHide();
      }}
    >
      <div className="overlay-panel" role="dialog" aria-label="Slop overlay">
        <header className="overlay-head">
          <span className="logo-text" aria-hidden="true">
            SL<b>O</b>P
          </span>
          {game && <span className="muted small">Playing {game}</span>}
          <nav className="overlay-tabs" aria-label="Overlay">
            <NavLink to="/chat" className={({ isActive }) => (isActive ? "on" : "")}>
              Chats
            </NavLink>
            <NavLink to="/friends" className={({ isActive }) => (isActive ? "on" : "")}>
              Friends
            </NavLink>
          </nav>
          <span className="grow" />
          <time className="muted small">{now.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}</time>
          <button className="btn small" onClick={() => void desktop.overlayHide()}>
            Back to the game <kbd>Shift+Tab</kbd>
          </button>
        </header>
        <div className="overlay-content">
          <Routes>
            <Route path="/chat" element={<ChatApp />} />
            <Route path="/chat/:id" element={<ChatApp />} />
            <Route path="/friends" element={<FriendsPage />} />
            <Route path="*" element={<Elsewhere />} />
          </Routes>
        </div>
      </div>
    </div>
  );
}

/** Links to other Slop pages: those open in the main window, not over the game. */
function Elsewhere() {
  const navigate = useNavigate();
  return (
    <div className="empty-state">
      That page opens in Slop, not over your game.{" "}
      <button className="link-btn" onClick={() => navigate("/chat")}>
        Back to chats
      </button>
    </div>
  );
}

document.documentElement.classList.add("overlay-root");
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <SessionProvider>
      <MemoryRouter initialEntries={["/chat"]}>
        <Overlay />
      </MemoryRouter>
    </SessionProvider>
  </StrictMode>,
);
