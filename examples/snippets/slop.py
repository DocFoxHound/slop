# Slop for Python games: saves, mods and asking the Slop app for a server's mods.
# Standard library only. Outside the Slop app it falls back: your own save folder, no mods,
# and request_mods answers "no_link".
import json, os, time

def save_dir(fallback):
    """Where saves, settings and logs go: Slop's save folder, else `fallback`."""
    path = os.environ.get("SLOP_SAVE_DIR") or fallback
    os.makedirs(path, exist_ok=True)
    return path

def save_json(fallback, name, data):
    """Saves safely: a new file first, renamed over the old one."""
    path = os.path.join(save_dir(fallback), name)
    with open(path + ".tmp", "w", encoding="utf-8") as f:
        json.dump(data, f)
    os.replace(path + ".tmp", path)

def load_json(fallback, name):
    """A save, or {} when it's missing, too big or not a JSON object."""
    path = os.path.join(save_dir(fallback), name)
    try:
        if os.path.getsize(path) > 1024 * 1024:
            return {}
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}

def mod_dirs(own_folder=None):
    """Every mod folder: your game's own mods folder (if you named one on your store page),
    then SLOP_MODS. Each holds one mod from Slop."""
    found, seen = [], set()
    if own_folder and os.path.isdir(own_folder):
        for name in sorted(os.listdir(own_folder)):
            if os.path.isdir(os.path.join(own_folder, name)):
                found.append(os.path.join(own_folder, name))
                seen.add(name)
    for d in filter(None, os.environ.get("SLOP_MODS", "").split(os.pathsep)):
        address = os.path.basename(os.path.dirname(d)) if os.path.basename(d) == "files" else os.path.basename(d)
        if address not in seen:
            seen.add(address)
            found.append(d)
    return found

def request_mods(mods, for_whom, give_up_after=600):
    """Asks the Slop app for mods by address, e.g. [{"slug": "dragons", "version": "1.2.0"}].
    The player confirms. Returns the answer: "done" (with "mods" and "folders"), "declined",
    "failed" (with "message"), "no_link" or "timeout"."""
    link = os.environ.get("SLOP_LINK")
    if not link:
        return {"status": "no_link"}
    name = "r%d" % time.time_ns()
    tmp = os.path.join(link, "requests", name + ".tmp")
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump({"mods": mods, "for": for_whom}, f)
    os.replace(tmp, os.path.join(link, "requests", name + ".json"))
    answer_path = os.path.join(link, "answers", name + ".json")
    until = time.monotonic() + give_up_after
    while time.monotonic() < until:
        time.sleep(0.5)
        try:
            with open(answer_path, encoding="utf-8") as f:
                answer = json.load(f)
        except (OSError, ValueError):
            continue
        if answer.get("status") != "waiting":
            return answer
    return {"status": "timeout"}
