// SPDX-License-Identifier: AGPL-3.0-or-later
// Chat (docs/DESIGN.md §19, SECURITY §8.5): direct messages, group chats and team chats, end-to-end
// encrypted in the Slop app's Rust core (crates/slop-client/src/chat). This page only shows what
// the app decrypts and asks it to send; keys never reach it. On the website, chat isn't
// available: browsers can't keep keys as safely, so the page explains and offers the app.
//
// Lightweight but complete: text with Markdown-lite, pictures, GIFs, videos and files (pasted,
// dropped or attached), links to memes shown inline, link previews made by the sender's app,
// replies, reactions, edits and deletes, mute, security codes, blocking and reporting.

import { useCallback, useEffect, useMemo, useRef, useState, type ClipboardEvent, type DragEvent, type FormEvent, type KeyboardEvent } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { api, ApiError, type FriendPerson, type ReportReason, type TeamCard } from "../api";
import { Loading } from "../components/ui";
import { useDocumentTitle } from "../hooks";
import { renderChat } from "../markdown";
import {
  desktop,
  isDesktop,
  openExternal,
  type ChatAttachment,
  type ChatConversation,
  type ChatMessage,
  type ChatStatus,
  detectOs,
  OS_LABEL,
} from "../platform";
import { useSession } from "../session";

const QUICK_REACTIONS = ["👍", "😂", "❤️", "🔥", "😮", "😢", "🎉", "💀"];
const EMOJI = ["😀", "😂", "🥲", "😍", "🤔", "😎", "😭", "😡", "👍", "👎", "🙏", "👀", "🔥", "💯", "🎉", "💀", "🤝", "🎮", "🕹️", "🏆", "🐛", "✨", "❤️", "💚"];
const MAX_FILE = 15 * 1024 * 1024;

const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));

/** Calls the app's chat engine. */
function chat<T>(action: Parameters<typeof desktop.chat>[0]) {
  return desktop.chat<T>(action);
}

export function ChatPage() {
  useDocumentTitle("Chat");
  const { me } = useSession();
  if (!isDesktop) return <ChatOnWeb />;
  if (me === undefined) return <Loading />;
  if (me === null) {
    return (
      <div className="page narrow">
        <div className="empty-state">
          <Link to="/signin?return_to=/chat">Sign in</Link> to chat.
        </div>
      </div>
    );
  }
  return <ChatApp />;
}

/** The website: chat is in the app only (SECURITY §8.5). Friends work here too. */
function ChatOnWeb() {
  const os = detectOs();
  return (
    <div className="page narrow">
      <h1 className="page-title">💬 Chat</h1>
      <div className="panel panel-pad">
        <p>
          <b>Chat is in the Slop app.</b> Messages are end-to-end encrypted on your devices, so Slop can't read them, and a
          website can't keep the keys as safely as the app can.
        </p>
        <p className="muted">
          Message friends, teammates and the people you work with on People; share pictures, GIFs and links; start group
          chats and team chats.
        </p>
        <div className="row-actions">
          <Link className="btn slime" to="/download">
            ⬇ Get Slop for {os === "mac" || os === "windows" || os === "linux" ? OS_LABEL[os] : "your computer"}
          </Link>
          <Link className="btn" to="/friends">
            Your friends
          </Link>
        </div>
      </div>
    </div>
  );
}

export function ChatApp() {
  const [status, setStatus] = useState<ChatStatus>();
  const [error, setError] = useState<string>();
  const load = useCallback(() => {
    chat<ChatStatus>({ action: "status" }).then(
      (s) => {
        setStatus(s);
        setError(undefined);
      },
      (e) => setError(errorText(e)),
    );
  }, []);
  useEffect(() => {
    load();
    let unlisten: (() => void) | undefined;
    void desktop.onChatChanged((c) => c.devices && load()).then((u) => (unlisten = u));
    // A pending device learns it was approved by asking.
    const timer = window.setInterval(load, 15000);
    return () => {
      unlisten?.();
      window.clearInterval(timer);
    };
  }, [load]);
  if (error) {
    return (
      <div className="page narrow">
        <p className="form-error" role="alert">
          {error}
        </p>
        <button className="btn" onClick={load}>
          Try again
        </button>
      </div>
    );
  }
  if (!status) return <Loading />;
  if (status.state === "none" || status.state === "revoked") return <SetUp status={status} onDone={setStatus} />;
  if (status.state === "pending") return <Waiting status={status} onDone={load} />;
  return <Chats status={status} reloadStatus={load} />;
}

// ---------------------------------------------------------------------------------------------
// Setting up this device

function SetUp({ status, onDone }: { status: ChatStatus; onDone: (s: ChatStatus) => void }) {
  const os = detectOs();
  const [name, setName] = useState(`Slop for ${OS_LABEL[os]}`);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [code, setCode] = useState<string>();
  const [result, setResult] = useState<ChatStatus>();
  const go = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      const s = await chat<ChatStatus>({ action: "setup", name: name.trim() });
      if (s.state === "active") {
        // The user's first device: backups are on by default, with a recovery code shown once.
        setCode((await chat<string | null>({ action: "set_backups", on: true })) ?? undefined);
        setResult(s);
      } else {
        onDone(s);
      }
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };
  if (result && code) return <RecoveryCode code={code} onDone={() => onDone({ ...result, backups: true })} />;
  return (
    <div className="page narrow">
      <h1 className="page-title">💬 Set up chat</h1>
      {status.state === "revoked" && (
        <p className="notice">This device was removed from chat. Set it up again to keep chatting here.</p>
      )}
      <form className="panel panel-pad form" onSubmit={(e) => void go(e)}>
        <p>
          Messages are encrypted on this device with keys only it has. Slop delivers them but can't read them, and neither can
          anyone who gets into your Slop account without one of your devices.
        </p>
        <label>
          Name this device
          <input value={name} maxLength={60} onChange={(e) => setName(e.target.value)} required />
        </label>
        <p className="muted small">If you already chat on another computer, you'll approve this one from there.</p>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <button className="btn slime" disabled={busy || !name.trim()}>
          {busy ? "Setting up…" : "Set up chat"}
        </button>
      </form>
    </div>
  );
}

function RecoveryCode({ code, onDone }: { code: string; onDone: () => void }) {
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  const off = async () => {
    setBusy(true);
    try {
      await chat({ action: "set_backups", on: false });
    } finally {
      setBusy(false);
      onDone();
    }
  };
  return (
    <div className="page narrow">
      <h1 className="page-title">Save your recovery code</h1>
      <div className="panel panel-pad">
        <p>
          Your chat history is backed up to Slop, encrypted with this code. Slop never sees it. If you lose every device you
          chat on, this code is the only way to get your history back.
        </p>
        <p className="recovery-code mono" aria-label="Recovery code">
          {code}
        </p>
        <p className="muted small">Write it down or keep it in a password manager. It won't be shown again.</p>
        <label className="check">
          <input type="checkbox" checked={saved} onChange={(e) => setSaved(e.target.checked)} />
          <span>I've saved my recovery code</span>
        </label>
        <div className="row-actions">
          <button className="btn slime" disabled={!saved} onClick={onDone}>
            Start chatting
          </button>
          <button className="btn ghost" disabled={busy} onClick={() => void off()}>
            Turn off backups instead
          </button>
        </div>
      </div>
    </div>
  );
}

function Waiting({ status, onDone }: { status: ChatStatus; onDone: () => void }) {
  const [startingOver, setStartingOver] = useState(false);
  const [error, setError] = useState<string>();
  const [restore, setRestore] = useState(false);
  const startOver = async () => {
    setError(undefined);
    try {
      await chat({ action: "start_over" });
      setRestore(true);
    } catch (err) {
      setError(errorText(err));
    }
  };
  if (restore) return <Restore onDone={onDone} />;
  return (
    <div className="page narrow">
      <h1 className="page-title">Approve this device</h1>
      <div className="panel panel-pad">
        <p>
          Open <b>Chat → Devices</b> in Slop on a computer where you already chat, and approve this one. Check the code there
          matches:
        </p>
        <p className="recovery-code mono" aria-label="This device's code">
          {status.code}
        </p>
        <p className="muted small">Once it's approved, your chats and their history arrive here by themselves.</p>
        {!startingOver ? (
          <button className="link-btn small" onClick={() => setStartingOver(true)}>
            I don't have my other device any more
          </button>
        ) : (
          <div className="notice">
            <p>
              Starting over makes this your only device for chat: your other devices are removed, and the people you chat
              with see that your security code changed. You can restore your history with your recovery code. It needs a
              recent sign-in.
            </p>
            <div className="row-actions">
              <button className="btn small danger" onClick={() => void startOver()}>
                Start over on this device
              </button>
              <button className="btn small ghost" onClick={() => setStartingOver(false)}>
                Never mind
              </button>
            </div>
          </div>
        )}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}

function Restore({ onDone }: { onDone: () => void }) {
  const [code, setCode] = useState("");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const go = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    try {
      await chat<number>({ action: "restore", code });
      onDone();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="page narrow">
      <h1 className="page-title">Restore your history</h1>
      <form className="panel panel-pad form" onSubmit={(e) => void go(e)}>
        <label>
          Recovery code
          <input value={code} onChange={(e) => setCode(e.target.value)} placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX" autoComplete="off" />
        </label>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <div className="row-actions">
          <button className="btn slime" disabled={busy || !code.trim()}>
            Restore
          </button>
          <button type="button" className="btn ghost" onClick={onDone}>
            Skip
          </button>
        </div>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// The chats

function Chats({ status, reloadStatus }: { status: ChatStatus; reloadStatus: () => void }) {
  const { id } = useParams();
  const navigate = useNavigate();
  const [list, setList] = useState<ChatConversation[]>();
  const [error, setError] = useState<string>();
  const [changed, setChanged] = useState<{ ids: string[]; n: number }>({ ids: [], n: 0 });
  const [panel, setPanel] = useState<"devices" | "new" | null>(null);
  // Who's typing: each entry lasts a few seconds unless renewed.
  const [typing, setTyping] = useState<Array<{ conversation: string; user: string; name: string; until: number }>>([]);
  const load = useCallback(() => {
    chat<ChatConversation[]>({ action: "conversations" }).then(setList, (e) => setError(errorText(e)));
  }, []);
  useEffect(() => {
    load();
    let unlisten: (() => void) | undefined;
    void desktop
      .onChatChanged((c) => {
        if (c.list || c.conversations.length) load();
        if (c.conversations.length) setChanged((p) => ({ ids: c.conversations, n: p.n + 1 }));
        setTyping((old) => {
          const now = Date.now();
          // A message from someone ends their "typing…".
          const sent = new Set((c.new_messages ?? []).map((m) => `${m.conversation}:${m.sender}`));
          const kept = old.filter((t) => t.until > now && !sent.has(`${t.conversation}:${t.name}`));
          for (const t of c.typing ?? []) {
            const i = kept.findIndex((k) => k.conversation === t.conversation && k.user === t.user);
            const entry = { ...t, until: now + 6000 };
            if (i >= 0) kept[i] = entry;
            else kept.push(entry);
          }
          return kept;
        });
      })
      .then((u) => (unlisten = u));
    const tick = window.setInterval(() => setTyping((old) => (old.some((t) => t.until <= Date.now()) ? old.filter((t) => t.until > Date.now()) : old)), 1000);
    return () => {
      unlisten?.();
      window.clearInterval(tick);
    };
  }, [load]);
  const current = list?.find((c) => c.id === id);
  useDocumentTitle(current ? `${current.title} · Chat` : "Chat");

  return (
    <div className="chat-layout">
      <aside className="chat-list" aria-label="Chats">
        <div className="chat-list-head">
          <h1>Chat</h1>
          <div className="row-actions">
            <button className="btn small slime" onClick={() => setPanel("new")}>
              ＋ New
            </button>
            <button className="btn small ghost" onClick={() => setPanel("devices")} aria-label="Devices and backups">
              ⚙{status.pending.length > 0 && <span className="dot-badge">{status.pending.length}</span>}
            </button>
          </div>
        </div>
        {status.pending.length > 0 && (
          <button className="notice chat-pending" onClick={() => setPanel("devices")}>
            A new device wants to join your chats. Check its code and approve it.
          </button>
        )}
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        {!list ? (
          <Loading />
        ) : list.length === 0 ? (
          <div className="empty-state small">
            No chats yet. <button className="link-btn" onClick={() => setPanel("new")}>Start one</button> with a friend or your team.
          </div>
        ) : (
          <ul>
            {list.map((c) => (
              <li key={c.id}>
                <Link to={`/chat/${c.id}`} className={`chat-item ${c.id === id ? "on" : ""}`} aria-current={c.id === id ? "page" : undefined}>
                  <span className="avatar" aria-hidden="true">
                    {c.kind === "team" ? "👥" : c.kind === "group" ? "#" : c.title.charAt(0).toUpperCase()}
                  </span>
                  <span className="chat-item-text">
                    <b>{c.title}</b>
                    <span className="muted small">{c.ready ? (c.last ?? "No messages yet") : "Waiting to be added…"}</span>
                  </span>
                  {c.muted && <span aria-label="Muted">🔕</span>}
                  {c.unread > 0 && (
                    <span className="unread-badge" aria-label={`${c.unread} unread`}>
                      {c.unread > 99 ? "99+" : c.unread}
                    </span>
                  )}
                </Link>
              </li>
            ))}
          </ul>
        )}
        <p className="muted small chat-list-foot">
          🔒 End-to-end encrypted. <Link to="/friends">Friends</Link>
        </p>
      </aside>
      <section className="chat-main">
        {current ? (
          <Conversation
            key={current.id}
            conv={current}
            changed={changed}
            typing={typing.filter((t) => t.conversation === current.id).map((t) => t.name)}
            onListChanged={load}
          />
        ) : (
          <div className="empty-state chat-empty">
            {list && list.length > 0 ? "Pick a chat." : "Your chats will show here."}
          </div>
        )}
      </section>
      {panel === "devices" && (
        <Panel title="Devices and backups" onClose={() => setPanel(null)}>
          <Devices status={status} onChanged={reloadStatus} />
        </Panel>
      )}
      {panel === "new" && (
        <Panel title="New chat" onClose={() => setPanel(null)}>
          <NewChat
            onStarted={(cid) => {
              setPanel(null);
              load();
              navigate(`/chat/${cid}`);
            }}
          />
        </Panel>
      )}
    </div>
  );
}

function Panel({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return (
    <dialog ref={ref} className="dialog" aria-label={title} onClose={onClose}>
      <div className="title-row spread">
        <h2>{title}</h2>
        <button className="btn small ghost" onClick={onClose} aria-label="Close">
          ✕
        </button>
      </div>
      {children}
    </dialog>
  );
}

function Devices({ status, onChanged }: { status: ChatStatus; onChanged: () => void }) {
  const [codes, setCodes] = useState<Record<string, string>>({});
  const [error, setError] = useState<string>();
  const [recovery, setRecovery] = useState<string>();
  const run = async (f: () => Promise<unknown>) => {
    setError(undefined);
    try {
      await f();
      onChanged();
    } catch (err) {
      setError(errorText(err));
    }
  };
  return (
    <div className="form">
      {status.pending.map((p) => (
        <div key={p.id} className="notice">
          <p>
            <b>{p.name}</b> wants to join your chats. Approve it only if it's yours: type the code it shows.
          </p>
          <label>
            Code on the new device
            <input
              className="mono"
              value={codes[p.id] ?? ""}
              onChange={(e) => setCodes({ ...codes, [p.id]: e.target.value })}
              placeholder="XXX-XXX-XXX"
              autoComplete="off"
            />
          </label>
          <div className="row-actions">
            <button className="btn small slime" onClick={() => void run(() => chat({ action: "approve", device: p.id, code: codes[p.id] ?? "" }))}>
              Approve
            </button>
            <button className="btn small ghost" onClick={() => void run(() => chat({ action: "remove_device", device: p.id }))}>
              It's not mine
            </button>
          </div>
        </div>
      ))}
      <h3>Your devices</h3>
      <ul className="report-list">
        {status.devices
          .filter((d) => d.status === "active")
          .map((d) => (
            <li key={d.id}>
              <span>
                <b>{d.name}</b> {d.this_device && <span className="muted small">· this device</span>}
              </span>
              {!d.this_device && (
                <button className="btn small ghost" onClick={() => void run(() => chat({ action: "remove_device", device: d.id }))}>
                  Remove
                </button>
              )}
            </li>
          ))}
      </ul>
      <h3>History backup</h3>
      {recovery ? (
        <>
          <p>Your new recovery code (the old one no longer works):</p>
          <p className="recovery-code mono">{recovery}</p>
        </>
      ) : status.backups ? (
        <>
          <p className="muted small">On: your history is backed up, encrypted with your recovery code. Slop can't read it.</p>
          <div className="row-actions">
            <button
              className="btn small"
              onClick={() => void run(async () => setRecovery((await chat<string | null>({ action: "set_backups", on: true })) ?? undefined))}
            >
              Make a new recovery code
            </button>
            <button className="btn small ghost" onClick={() => void run(() => chat({ action: "set_backups", on: false }))}>
              Turn off and delete the backup
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="muted small">Off: if you lose every device, your history is gone.</p>
          <button
            className="btn small"
            onClick={() => void run(async () => setRecovery((await chat<string | null>({ action: "set_backups", on: true })) ?? undefined))}
          >
            Turn on backups
          </button>
        </>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

function NewChat({ onStarted }: { onStarted: (id: string) => void }) {
  const [tab, setTab] = useState<"direct" | "group" | "team">("direct");
  const [friends, setFriends] = useState<FriendPerson[]>();
  const [teams, setTeams] = useState<TeamCard[]>();
  const [picked, setPicked] = useState<string[]>([]);
  const [name, setName] = useState("");
  const [link, setLink] = useState("");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.friends.list().then((f) => setFriends(f.friends), () => setFriends([]));
    api.teams.mine().then((t) => setTeams(t.teams.filter((x) => x.kind === "player")), () => setTeams([]));
  }, []);
  const start = async (action: Parameters<typeof desktop.chat>[0]) => {
    setBusy(true);
    setError(undefined);
    try {
      onStarted(await chat<string>(action));
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="form">
      <div className="seg" role="tablist">
        {(["direct", "group", "team"] as const).map((t) => (
          <button key={t} role="tab" aria-selected={tab === t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>
            {t === "direct" ? "Message someone" : t === "group" ? "Group chat" : "Team chat"}
          </button>
        ))}
      </div>
      {tab === "direct" && (
        <>
          {friends && friends.length > 0 && (
            <ul className="pick-list">
              {friends.map((f) => (
                <li key={f.id}>
                  <button className="btn small" disabled={busy} onClick={() => void start({ action: "start_direct", user: f.id })}>
                    {f.name}
                  </button>
                </li>
              ))}
            </ul>
          )}
          <label>
            Or paste someone's profile link
            <input value={link} onChange={(e) => setLink(e.target.value)} placeholder="https://…/u/…" />
          </label>
          <p className="muted small">
            You can message friends, teammates and the people you work with on People, and anyone who takes messages from
            everyone.
          </p>
          <button className="btn slime" disabled={busy || !link.trim()} onClick={() => void start({ action: "start_direct", user: link.trim() })}>
            Start chat
          </button>
        </>
      )}
      {tab === "group" && (
        <>
          <label>
            Group name (only the people in it see it)
            <input value={name} maxLength={80} onChange={(e) => setName(e.target.value)} placeholder="Friday night roguelikes" />
          </label>
          {friends?.length === 0 && <p className="muted">Add friends first: groups are made from people you're connected to.</p>}
          <fieldset className="pick-list">
            <legend className="small muted">Who's in it (up to 49 others)</legend>
            {friends?.map((f) => (
              <label className="check" key={f.id}>
                <input
                  type="checkbox"
                  checked={picked.includes(f.id)}
                  onChange={(e) => setPicked(e.target.checked ? [...picked, f.id] : picked.filter((p) => p !== f.id))}
                />
                <span>{f.name}</span>
              </label>
            ))}
          </fieldset>
          <button
            className="btn slime"
            disabled={busy || picked.length === 0}
            onClick={() => void start({ action: "start_group", users: picked, name: name.trim() || null })}
          >
            Start group chat
          </button>
        </>
      )}
      {tab === "team" && (
        <>
          {teams?.length === 0 && (
            <p className="muted">
              You're not in a player team yet. <Link to="/teams">Find or start one</Link>.
            </p>
          )}
          <ul className="pick-list">
            {teams?.map((t) => (
              <li key={t.slug}>
                <button className="btn small" disabled={busy} onClick={() => void start({ action: "start_team", team: t.slug })}>
                  👥 {t.name}
                </button>
              </li>
            ))}
          </ul>
          <p className="muted small">Everyone in the team is in its chat, and it follows who joins and leaves.</p>
        </>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// One chat

function Conversation({
  conv,
  changed,
  typing,
  onListChanged,
}: {
  conv: ChatConversation;
  changed: { ids: string[]; n: number };
  typing: string[];
  onListChanged: () => void;
}) {
  const navigate = useNavigate();
  const { me } = useSession();
  const [messages, setMessages] = useState<ChatMessage[]>();
  const [older, setOlder] = useState<ChatMessage[]>([]);
  const [more, setMore] = useState(true);
  const [error, setError] = useState<string>();
  const [replyTo, setReplyTo] = useState<ChatMessage>();
  const [editing, setEditing] = useState<ChatMessage>();
  const [reporting, setReporting] = useState<ChatMessage>();
  const [link, setLink] = useState<string>();
  const [menu, setMenu] = useState<"codes" | "add" | null>(null);
  const log = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);

  const load = useCallback(() => {
    chat<ChatMessage[]>({ action: "messages", conversation: conv.id }).then(
      (m) => {
        setMessages(m);
        if (m.length < 200) setMore(false);
      },
      (e) => setError(errorText(e)),
    );
    void chat({ action: "mark_read", conversation: conv.id }).then(() => {
      onListChanged();
      window.dispatchEvent(new Event("slop-chat-read"));
    }, () => undefined);
  }, [conv.id, onListChanged]);
  useEffect(load, [load]);
  useEffect(() => {
    if (changed.ids.includes(conv.id)) load();
  }, [changed, conv.id, load]);
  // Stay at the newest message unless the reader scrolled up.
  useEffect(() => {
    if (atBottom.current && log.current) log.current.scrollTop = log.current.scrollHeight;
  }, [messages]);

  const loadOlder = async () => {
    const first = (older[0] ?? messages?.[0])?.seq;
    if (first === undefined) return;
    const page = await chat<ChatMessage[]>({ action: "messages", conversation: conv.id, before: first });
    if (page.length < 200) setMore(false);
    setOlder([...page, ...older]);
  };

  const all = useMemo(() => [...older, ...(messages ?? [])], [older, messages]);

  const run = async (f: () => Promise<unknown>) => {
    setError(undefined);
    try {
      await f();
      load();
    } catch (err) {
      setError(errorText(err));
    }
  };

  // Links in messages: say where they go before opening them (SECURITY §8.5).
  const onClick = (e: React.MouseEvent) => {
    const a = (e.target as HTMLElement).closest("a[data-external]");
    if (!a) return;
    e.preventDefault();
    setLink(a.getAttribute("href") ?? undefined);
  };

  const other = conv.kind === "direct" ? conv.members.find((m) => m.id !== me?.id) : undefined;

  return (
    <div className="conversation">
      <header className="conversation-head">
        <div>
          <h2>{conv.title}</h2>
          <p className="muted small">
            {conv.kind === "team" && conv.team_slug ? (
              <Link to={`/teams/${conv.team_slug}`}>Team chat · {conv.members.length} members</Link>
            ) : conv.kind === "group" ? (
              conv.members.map((m) => m.name).join(", ")
            ) : (
              other && <Link to={`/u/${other.id}`}>Profile</Link>
            )}{" "}
            · 🔒 end-to-end encrypted
          </p>
        </div>
        <div className="row-actions">
          <button className="btn small ghost" onClick={() => setMenu("codes")}>
            Security codes
          </button>
          <button className="btn small ghost" onClick={() => void run(() => chat({ action: "mute", conversation: conv.id, muted: !conv.muted }).then(onListChanged))}>
            {conv.muted ? "Unmute" : "Mute"}
          </button>
          {conv.kind === "group" && (
            <>
              <button className="btn small ghost" onClick={() => setMenu("add")}>
                Add people
              </button>
              <button
                className="btn small ghost"
                onClick={() =>
                  void run(async () => {
                    await chat({ action: "leave", conversation: conv.id });
                    onListChanged();
                    navigate("/chat");
                  })
                }
              >
                Leave
              </button>
            </>
          )}
          {conv.kind !== "group" && (
            <button
              className="btn small ghost"
              onClick={() =>
                void run(async () => {
                  await chat({ action: "hide", conversation: conv.id });
                  onListChanged();
                  navigate("/chat");
                })
              }
            >
              Hide
            </button>
          )}
        </div>
      </header>
      <div
        className="message-log"
        ref={log}
        role="log"
        aria-live="polite"
        aria-label={`Messages with ${conv.title}`}
        onClick={onClick}
        // Pictures load after the text: keep the newest message in view if the reader was there.
        onLoadCapture={() => {
          if (atBottom.current && log.current) log.current.scrollTop = log.current.scrollHeight;
        }}
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
      >
        {more && messages && messages.length >= 200 && (
          <button className="btn small ghost load-older" onClick={() => void loadOlder()}>
            Load earlier messages
          </button>
        )}
        {!conv.ready && (
          <p className="notice">
            Waiting to be added to this chat's encryption. Someone in it adds this device next time their Slop app is open.
          </p>
        )}
        {!messages ? (
          <Loading />
        ) : all.length === 0 && conv.ready ? (
          <p className="muted chat-hello">Say hi! Messages here are end-to-end encrypted.</p>
        ) : (
          all.map((m, i) => (
            <MessageRow
              key={`${m.id}-${m.seq}`}
              conv={conv}
              m={m}
              grouped={i > 0 && !all[i - 1].notice && all[i - 1].sender.id === m.sender.id && !m.notice}
              onReply={() => setReplyTo(m)}
              onEdit={() => setEditing(m)}
              onDelete={() => void run(() => chat({ action: "delete", conversation: conv.id, message: m.id }))}
              onReact={(emoji, on) => void run(() => chat({ action: "react", conversation: conv.id, message: m.id, emoji, on }))}
              onReport={() => setReporting(m)}
            />
          ))
        )}
      </div>
      {error && (
        <p className="form-error chat-error" role="alert">
          {error}
        </p>
      )}
      <p className="typing small muted" aria-live="polite">
        {typing.length === 1
          ? `${typing[0]} is typing…`
          : typing.length === 2
            ? `${typing[0]} and ${typing[1]} are typing…`
            : typing.length > 2
              ? "Several people are typing…"
              : "\u00a0"}
      </p>
      {conv.closed ? (
        <p className="notice chat-closed">You can't message {conv.title} any more.</p>
      ) : (
        <Composer
          key={editing?.id ?? "new"}
          conv={conv}
          replyTo={replyTo}
          editing={editing}
          onCancel={() => {
            setReplyTo(undefined);
            setEditing(undefined);
          }}
          onSent={() => {
            setReplyTo(undefined);
            setEditing(undefined);
            atBottom.current = true;
            load();
          }}
        />
      )}
      {link && <LinkWarning url={link} onClose={() => setLink(undefined)} />}
      {reporting && <ReportMessage conv={conv} m={reporting} onClose={() => setReporting(undefined)} />}
      {menu === "codes" && (
        <Panel title="Security codes" onClose={() => setMenu(null)}>
          <SecurityCodes conv={conv} />
        </Panel>
      )}
      {menu === "add" && (
        <Panel title="Add people" onClose={() => setMenu(null)}>
          <AddPeople
            conv={conv}
            onDone={() => {
              setMenu(null);
              onListChanged();
            }}
          />
        </Panel>
      )}
    </div>
  );
}

function time(iso: string) {
  if (!iso) return "";
  const d = new Date(iso);
  const today = new Date().toDateString() === d.toDateString();
  return today
    ? d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
    : d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

function MessageRow({
  conv,
  m,
  grouped,
  onReply,
  onEdit,
  onDelete,
  onReact,
  onReport,
}: {
  conv: ChatConversation;
  m: ChatMessage;
  grouped: boolean;
  onReply: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onReact: (emoji: string, on: boolean) => void;
  onReport: () => void;
}) {
  const [picking, setPicking] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  if (m.notice) {
    return (
      <p className="chat-notice" role="note">
        🔒 {m.text}
      </p>
    );
  }
  const html = m.text ? renderChat(m.text) : "";
  return (
    <article className={`message ${m.mine ? "mine" : ""} ${grouped ? "grouped" : ""}`} aria-label={`${m.sender.name}, ${time(m.at)}`}>
      {!grouped && (
        <div className="message-meta">
          <b>{m.sender.name}</b> <time className="muted small">{time(m.at)}</time>
        </div>
      )}
      {m.reply_to && (
        <div className="message-reply small">
          ↪ <b>{m.reply_to.sender}</b>: {m.reply_to.text}
        </div>
      )}
      {m.deleted ? (
        <p className="muted small">
          <i>Message deleted</i>
        </p>
      ) : (
        <>
          {html && <div className="message-body" dangerouslySetInnerHTML={{ __html: html }} />}
          {m.attachments.map((a) => (
            <Attachment key={a.id} conv={conv} m={m} a={a} />
          ))}
          {m.preview && (
            <a className="link-preview" href={m.preview.url} data-external="">
              {m.preview.image && <Attachment conv={conv} m={m} a={m.preview.image} thumb />}
              <span>
                <span className="muted small">{m.preview.site}</span>
                {m.preview.title && <b>{m.preview.title}</b>}
                {m.preview.description && <span className="small">{m.preview.description}</span>}
              </span>
            </a>
          )}
          {m.edited && <span className="muted small"> (edited)</span>}
        </>
      )}
      {m.warnings.map((w) => (
        <p key={w} className="notice small chat-warning" role="note">
          ⚠ {w}
        </p>
      ))}
      {m.reactions.length > 0 && (
        <div className="reactions">
          {m.reactions.map((r) => (
            <button
              key={r.emoji}
              className={`reaction ${r.mine ? "on" : ""}`}
              title={r.names.join(", ")}
              aria-pressed={r.mine}
              aria-label={`${r.emoji} ${r.count}: ${r.names.join(", ")}`}
              onClick={() => onReact(r.emoji, !r.mine)}
            >
              {r.emoji} {r.count}
            </button>
          ))}
        </div>
      )}
      {!m.deleted && (
        <div className="message-actions">
          <button className="link-btn small" onClick={() => setPicking(!picking)} aria-expanded={picking}>
            React
          </button>
          <button className="link-btn small" onClick={onReply}>
            Reply
          </button>
          {m.mine && (
            <>
              {m.text && (
                <button className="link-btn small" onClick={onEdit}>
                  Edit
                </button>
              )}
              {confirmDelete ? (
                <>
                  <button className="link-btn small danger-text" onClick={onDelete}>
                    Delete for everyone
                  </button>
                  <button className="link-btn small" onClick={() => setConfirmDelete(false)}>
                    Keep
                  </button>
                </>
              ) : (
                <button className="link-btn small" onClick={() => setConfirmDelete(true)}>
                  Delete
                </button>
              )}
            </>
          )}
          {m.reportable && (
            <button className="link-btn small muted" onClick={onReport}>
              ⚑ Report
            </button>
          )}
        </div>
      )}
      {picking && (
        <div className="emoji-row" role="group" aria-label="Pick a reaction">
          {QUICK_REACTIONS.map((e) => (
            <button
              key={e}
              className="emoji-btn"
              onClick={() => {
                setPicking(false);
                onReact(e, !m.reactions.some((r) => r.emoji === e && r.mine));
              }}
            >
              {e}
            </button>
          ))}
        </div>
      )}
    </article>
  );
}

/** Decrypted attachments, kept for the session so scrolling back doesn't fetch again. */
const blobs = new Map<string, string>();

function Attachment({ conv, m, a, thumb = false }: { conv: ChatConversation; m: ChatMessage; a: ChatAttachment; thumb?: boolean }) {
  const media = a.mime.startsWith("image/") || a.mime.startsWith("video/");
  const [url, setUrl] = useState(blobs.get(a.id));
  const [error, setError] = useState<string>();
  const fetchIt = useCallback(async () => {
    try {
      const got = await chat<{ mime: string; data: string }>({ action: "attachment", conversation: conv.id, message: m.id, attachment: a.id });
      const bytes = Uint8Array.from(atob(got.data), (c) => c.charCodeAt(0));
      const u = URL.createObjectURL(new Blob([bytes], { type: got.mime }));
      blobs.set(a.id, u);
      return u;
    } catch (err) {
      setError(errorText(err));
      return undefined;
    }
  }, [conv.id, m.id, a.id]);
  useEffect(() => {
    if (media && !url) void fetchIt().then(setUrl);
  }, [media, url, fetchIt]);
  if (error) return <p className="muted small">📎 {a.name}: {error}</p>;
  if (!media) {
    const download = async () => {
      const u = url ?? (await fetchIt());
      if (!u) return;
      const link = document.createElement("a");
      link.href = u;
      link.download = a.name || "attachment";
      link.click();
    };
    return (
      <button className="file-chip" onClick={() => void download()}>
        📎 {a.name} <span className="muted small">{(a.size / 1024).toFixed(a.size < 102400 ? 1 : 0)} KB</span>
      </button>
    );
  }
  if (!url) return <div className={`media-placeholder ${thumb ? "thumb" : ""}`} aria-label="Loading picture" />;
  if (a.mime.startsWith("video/")) {
    return <video className="chat-media" src={url} controls loop muted playsInline aria-label={a.name} />;
  }
  return <img className={thumb ? "preview-thumb" : "chat-media"} src={url} alt={a.source_url ? `Picture from ${a.source_url}` : a.name} loading="lazy" />;
}

function readFile(f: File): Promise<{ name: string; mime: string; data: string }> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => {
      const s = String(r.result);
      resolve({ name: f.name || "pasted", mime: f.type || "application/octet-stream", data: s.slice(s.indexOf(",") + 1) });
    };
    r.onerror = () => reject(new Error(`couldn't read ${f.name}`));
    r.readAsDataURL(f);
  });
}

function loadPreviewSetting(): boolean {
  try {
    return localStorage.getItem("slop.chat.previews") !== "off";
  } catch {
    return true;
  }
}

function Composer({
  conv,
  replyTo,
  editing,
  onCancel,
  onSent,
}: {
  conv: ChatConversation;
  replyTo?: ChatMessage;
  editing?: ChatMessage;
  onCancel: () => void;
  onSent: () => void;
}) {
  const [text, setText] = useState(editing?.text ?? "");
  const [files, setFiles] = useState<File[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [emoji, setEmoji] = useState(false);
  const [previews, setPreviews] = useState(loadPreviewSetting);
  const [dragging, setDragging] = useState(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  // "Typing…" for the others: at most every 3 seconds while typing, never while editing.
  const lastTyping = useRef(0);
  const signalTyping = (value: string) => {
    if (editing || !value.trim()) return;
    const now = Date.now();
    if (now - lastTyping.current < 3000) return;
    lastTyping.current = now;
    void chat({ action: "typing", conversation: conv.id, typing: true }).catch(() => undefined);
  };
  useEffect(() => input.current?.focus(), [replyTo, editing]);

  const addFiles = (list: FileList | File[]) => {
    const add = Array.from(list);
    const big = add.find((f) => f.size > MAX_FILE);
    if (big) {
      setError(`${big.name} is too big: files are 15 MB at most.`);
      return;
    }
    setFiles((f) => [...f, ...add].slice(0, 10));
  };

  const send = async (e?: FormEvent) => {
    e?.preventDefault();
    if (busy || (!text.trim() && files.length === 0)) return;
    setBusy(true);
    setError(undefined);
    try {
      if (editing) {
        await chat({ action: "edit", conversation: conv.id, message: editing.id, text });
      } else {
        await chat({
          action: "send",
          conversation: conv.id,
          text,
          reply_to: replyTo?.id ?? null,
          files: await Promise.all(files.map(readFile)),
          link_preview: previews,
        });
      }
      setText("");
      setFiles([]);
      onSent();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };
  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void send();
    }
    if (e.key === "Escape") onCancel();
  };
  const onPaste = (e: ClipboardEvent) => {
    if (e.clipboardData.files.length > 0) {
      e.preventDefault();
      addFiles(e.clipboardData.files);
    }
  };
  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setDragging(false);
    if (e.dataTransfer.files.length > 0) addFiles(e.dataTransfer.files);
  };
  const togglePreviews = (on: boolean) => {
    setPreviews(on);
    try {
      localStorage.setItem("slop.chat.previews", on ? "on" : "off");
    } catch {
      // Not remembered; it still applies now.
    }
  };

  return (
    <form
      className={`composer ${dragging ? "dragging" : ""}`}
      onSubmit={(e) => void send(e)}
      onDragOver={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={onDrop}
    >
      {(replyTo || editing) && (
        <div className="composer-context small">
          {editing ? "Editing your message" : `Replying to ${replyTo!.sender.name}: ${replyTo!.text.slice(0, 80) || "📎 Attachment"}`}
          <button type="button" className="link-btn small" onClick={onCancel}>
            Cancel
          </button>
        </div>
      )}
      {files.length > 0 && (
        <ul className="composer-files">
          {files.map((f, i) => (
            <li key={`${f.name}-${i}`} className="file-chip">
              📎 {f.name}
              <button type="button" className="link-btn small" aria-label={`Remove ${f.name}`} onClick={() => setFiles(files.filter((_, j) => j !== i))}>
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="composer-row">
        {!editing && (
          <>
            <button type="button" className="btn small ghost" aria-label="Attach pictures, GIFs, videos or files" onClick={() => fileInput.current?.click()}>
              📎
            </button>
            <input
              ref={fileInput}
              type="file"
              multiple
              hidden
              onChange={(e) => {
                if (e.target.files) addFiles(e.target.files);
                e.target.value = "";
              }}
            />
          </>
        )}
        <button type="button" className="btn small ghost" aria-label="Emoji" aria-expanded={emoji} onClick={() => setEmoji(!emoji)}>
          😀
        </button>
        <textarea
          ref={input}
          value={text}
          rows={Math.min(6, Math.max(1, text.split("\n").length))}
          maxLength={4000}
          placeholder={conv.ready ? `Message ${conv.title}` : "Waiting to be added to this chat…"}
          aria-label={`Message ${conv.title}`}
          disabled={!conv.ready}
          onChange={(e) => {
            setText(e.target.value);
            signalTyping(e.target.value);
          }}
          onKeyDown={onKey}
          onPaste={onPaste}
        />
        <button className="btn slime" disabled={busy || !conv.ready || (!text.trim() && files.length === 0)}>
          {busy ? "…" : editing ? "Save" : "Send"}
        </button>
      </div>
      {emoji && (
        <div className="emoji-row" role="group" aria-label="Insert an emoji">
          {EMOJI.map((e) => (
            <button
              type="button"
              key={e}
              className="emoji-btn"
              onClick={() => {
                setText((t) => t + e);
                input.current?.focus();
              }}
            >
              {e}
            </button>
          ))}
        </div>
      )}
      <div className="composer-foot muted small">
        <label className="check">
          <input type="checkbox" checked={previews} onChange={(e) => togglePreviews(e.target.checked)} />
          <span>Link previews (your app fetches the link, never the recipients')</span>
        </label>
        <span>Enter sends · Shift+Enter for a new line · paste or drop pictures</span>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}

/** Before opening a link from a message: where it really goes (SECURITY §8.5). */
function LinkWarning({ url, onClose }: { url: string; onClose: () => void }) {
  let host = "";
  try {
    host = new URL(url).host;
  } catch {
    host = "";
  }
  return (
    <Panel title="Open this link?" onClose={onClose}>
      {host ? (
        <>
          <p>
            This link goes to <b className="mono">{host}</b>, outside Slop. Only open it if you trust who sent it: links in
            chats are a common way to steal accounts.
          </p>
          <p className="muted small mono break">{url}</p>
          <div className="row-actions">
            <button
              className="btn slime"
              onClick={() => {
                void openExternal(url);
                onClose();
              }}
            >
              Open in browser
            </button>
            <button className="btn ghost" onClick={onClose}>
              Cancel
            </button>
          </div>
        </>
      ) : (
        <p>That link doesn't look right, so Slop won't open it.</p>
      )}
    </Panel>
  );
}

function SecurityCodes({ conv }: { conv: ChatConversation }) {
  const [codes, setCodes] = useState<Array<{ user: string; name: string; code: string }>>();
  useEffect(() => {
    chat<Array<{ user: string; name: string; code: string }>>({ action: "security_codes", conversation: conv.id }).then(setCodes, () => setCodes([]));
  }, [conv.id]);
  return (
    <div>
      <p className="muted small">
        Each person's code comes from their own devices' keys. Compare them in person or on a call: if they match on both
        screens, nobody is in between. If someone sets chat up again on a new device, their code changes and the chat says
        so.
      </p>
      {!codes ? (
        <Loading />
      ) : (
        <ul className="report-list">
          {codes.map((c) => (
            <li key={c.user}>
              <b>{c.name}</b>
              <span className="mono">{c.code}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function AddPeople({ conv, onDone }: { conv: ChatConversation; onDone: () => void }) {
  const [friends, setFriends] = useState<FriendPerson[]>();
  const [picked, setPicked] = useState<string[]>([]);
  const [error, setError] = useState<string>();
  useEffect(() => {
    api.friends.list().then((f) => setFriends(f.friends.filter((x) => !conv.members.some((m) => m.id === x.id))), () => setFriends([]));
  }, [conv.members]);
  const add = async () => {
    try {
      await chat({ action: "add_people", conversation: conv.id, users: picked });
      onDone();
    } catch (err) {
      setError(errorText(err));
    }
  };
  return (
    <div className="form">
      {friends?.length === 0 && <p className="muted">All your friends are already here.</p>}
      {friends?.map((f) => (
        <label className="check" key={f.id}>
          <input
            type="checkbox"
            checked={picked.includes(f.id)}
            onChange={(e) => setPicked(e.target.checked ? [...picked, f.id] : picked.filter((p) => p !== f.id))}
          />
          <span>{f.name}</span>
        </label>
      ))}
      <p className="muted small">They'll see messages sent from now on, not earlier ones.</p>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      <button className="btn slime" disabled={picked.length === 0} onClick={() => void add()}>
        Add
      </button>
    </div>
  );
}

/** Reporting a message: the app sends Slop this one message, decrypted, with proof it's genuine. */
function ReportMessage({ conv, m, onClose }: { conv: ChatConversation; m: ChatMessage; onClose: () => void }) {
  const [reasons, setReasons] = useState<ReportReason[]>();
  const [reason, setReason] = useState("");
  const [details, setDetails] = useState("");
  const [goodFaith, setGoodFaith] = useState(false);
  const [error, setError] = useState<string>();
  const [done, setDone] = useState<string>();
  const [block, setBlock] = useState(false);
  useEffect(() => {
    api.reports.reasons("chat_message").then(setReasons, (e: unknown) => e instanceof ApiError && setError(e.message));
  }, []);
  const send = async (e: FormEvent) => {
    e.preventDefault();
    setError(undefined);
    try {
      const ack = await chat<{ reference: string }>({ action: "report", conversation: conv.id, message: m.id, reason, details });
      if (block && !m.mine) await api.friends.block(m.sender.id).catch(() => undefined);
      setDone(ack.reference);
    } catch (err) {
      setError(errorText(err));
    }
  };
  return (
    <Panel title={`Report a message from ${m.sender.name}`} onClose={onClose}>
      {done ? (
        <>
          <p>
            Thanks. Your reference is <b className="mono">{done}</b>; you'll see the outcome on your account page.
          </p>
          <button className="btn" onClick={onClose}>
            Close
          </button>
        </>
      ) : (
        <form className="form" onSubmit={(e) => void send(e)}>
          <p className="muted small">
            Slop can't read chats. Reporting sends this one message to Slop's moderators, with a check that proves it's
            exactly what {m.sender.name} sent. Nothing else from the chat is shared.
          </p>
          <blockquote className="quoted small">{m.text || "📎 Attachment"}</blockquote>
          <label>
            What's wrong
            <select value={reason} onChange={(e) => setReason(e.target.value)} required>
              <option value="">Choose…</option>
              {reasons?.map((r) => (
                <option key={r.code} value={r.code}>
                  {r.label}
                </option>
              ))}
            </select>
          </label>
          <label>
            Details
            <textarea value={details} onChange={(e) => setDetails(e.target.value)} maxLength={4000} required rows={3} />
          </label>
          <label className="check">
            <input type="checkbox" checked={goodFaith} onChange={(e) => setGoodFaith(e.target.checked)} />
            <span>I believe this report is accurate and complete.</span>
          </label>
          <label className="check">
            <input type="checkbox" checked={block} onChange={(e) => setBlock(e.target.checked)} />
            <span>Also block {m.sender.name}</span>
          </label>
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <button className="btn danger" disabled={!reason || !details.trim() || !goodFaith}>
            Send report
          </button>
        </form>
      )}
    </Panel>
  );
}
