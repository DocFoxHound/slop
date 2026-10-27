# SLAPorSLOP for Godot 4: saves, mods, mod requests (Slop Link) and player counts (Slop Pulse).
# Add this file as an autoload named "Slop" (Project → Project Settings → Globals). Copy it into
# your own game as it is; nothing here is specific to Pong. See https://slop.example/publish/guide.
#
# Outside the SLAPorSLOP app none of the variables are set, and everything falls back: saves go to
# user://, there are no mods from SLAPorSLOP, mod requests answer "no_link", and Pulse does nothing.
extends Node

## Mods loaded by load_mods(): one Dictionary per mod, as its mod.json says, plus "dir".
var mods: Array[Dictionary] = []

const MAX_JSON_BYTES := 1024 * 1024

func _ready() -> void:
	_start_pulse()


# ---------------------------------------------------------------------------------------------
# Saves: everything the game keeps on the player's computer goes in SLOP_SAVE_DIR, the same
# folder in normal and sandbox mode.

## The folder for saves, settings and logs.
func save_dir() -> String:
	var dir := OS.get_environment("SLOP_SAVE_DIR")
	return dir if not dir.is_empty() else "user://"


## Saves a Dictionary as JSON, safely: a new file first, then renamed over the old one, so a
## crash never leaves half a save.
func save_json(file_name: String, data: Dictionary) -> bool:
	var path := save_dir().path_join(file_name)
	var tmp := path + ".tmp"
	var f := FileAccess.open(tmp, FileAccess.WRITE)
	if f == null:
		return false
	f.store_string(JSON.stringify(data, "\t"))
	f.close()
	return DirAccess.rename_absolute(tmp, path) == OK


## Loads a save; {} when it's missing, too big or not a JSON object.
func load_json(file_name: String) -> Dictionary:
	return _read_json(save_dir().path_join(file_name))


func _read_json(path: String) -> Dictionary:
	if not FileAccess.file_exists(path):
		return {}
	var f := FileAccess.open(path, FileAccess.READ)
	if f == null or f.get_length() > MAX_JSON_BYTES:
		return {}
	var parsed = JSON.parse_string(f.get_as_text())
	return parsed if parsed is Dictionary else {}


# ---------------------------------------------------------------------------------------------
# Mods: only mods from SLAPorSLOP. The store page names "mods" next to the game as the mods folder
# (each mod in its own folder); the SLAPorSLOP app keeps exactly the approved mods there. SLOP_MODS
# lists the same folders, for games that don't name one.

## Every mod folder: the game's own mods folder, then SLOP_MODS. Each holds one mod.
func mod_dirs() -> PackedStringArray:
	var found := PackedStringArray()
	# Next to the exported game. (Not in the editor: there it would be next to Godot itself.)
	if OS.has_feature("template"):
		var own := OS.get_executable_path().get_base_dir().path_join("mods")
		for sub in DirAccess.get_directories_at(own):
			found.append(own.path_join(sub))
	# The same mod is in both lists: <mods folder>/<address> and <SLAPorSLOP's copy>/<address>/files.
	var seen := {}
	for f in found:
		seen[f.get_file()] = true
	var sep := ";" if OS.get_name() == "Windows" else ":"
	for dir in OS.get_environment("SLOP_MODS").split(sep, false):
		var address := dir.get_base_dir().get_file() if dir.get_file() == "files" else dir.get_file()
		if not seen.has(address):
			seen[address] = true
			found.append(dir)
	return found


## Reads each mod's mod.json and, if it has one, creates its script. Returns the mods loaded.
## A mod.json: {"name": "Neon", "script": "mod.gd", ...anything the game reads}.
func load_mods() -> Array[Dictionary]:
	mods.clear()
	for dir in mod_dirs():
		var info := _read_json(dir.path_join("mod.json"))
		if info.is_empty() or not info.get("name") is String:
			push_warning("Slop: skipping %s: no valid mod.json" % dir)
			continue
		info["dir"] = dir
		var script_name = info.get("script")
		if script_name is String and _plain_name(script_name):
			# Mods may carry code: SLAPorSLOP reviewed it, and it only ever comes from SLAPorSLOP.
			var script = load(dir.path_join(script_name))
			if script is GDScript:
				info["instance"] = script.new()
		mods.append(info)
	return mods


func _plain_name(name: String) -> bool:
	return name.is_valid_filename() and not name.begins_with(".")


# ---------------------------------------------------------------------------------------------
# Slop Link: ask the SLAPorSLOP app for mods a server or lobby needs. The player confirms; SLAPorSLOP
# downloads and checks them and adds them to the game. Servers only name mods.

## Asks for mods by SLAPorSLOP address and, optionally, version:
##   await Slop.request_mods([{"slug": "slop-pong-neon"}], "Test server")
## Returns the answer: "done" (with "mods" and "folders"), "declined", "failed" (with
## "message"), "no_link" (not started from SLAPorSLOP) or "timeout".
func request_mods(wanted: Array, for_whom: String, give_up_after := 600.0) -> Dictionary:
	var link := OS.get_environment("SLOP_LINK")
	if link.is_empty():
		return {"status": "no_link"}
	var name := "r%d" % Time.get_ticks_usec()
	var tmp := link.path_join("requests/%s.tmp" % name)
	var f := FileAccess.open(tmp, FileAccess.WRITE)
	if f == null:
		return {"status": "failed", "message": "couldn't write the request"}
	f.store_string(JSON.stringify({"mods": wanted, "for": for_whom}))
	f.close()
	DirAccess.rename_absolute(tmp, link.path_join("requests/%s.json" % name))
	var answer_path := link.path_join("answers/%s.json" % name)
	var waited := 0.0
	while waited < give_up_after:
		await get_tree().create_timer(0.5).timeout
		waited += 0.5
		var answer := _read_json(answer_path)
		if not answer.is_empty() and answer.get("status") != "waiting":
			return answer
	return {"status": "timeout"}


# ---------------------------------------------------------------------------------------------
# Slop Pulse: player counts. Sends the session token and nothing else.

func _start_pulse() -> void:
	var token := OS.get_environment("SLOP_PULSE_TOKEN")
	var url := OS.get_environment("SLOP_PULSE_URL")
	if token.is_empty() or url.is_empty():
		return
	var every := OS.get_environment("SLOP_PULSE_INTERVAL")
	var timer := Timer.new()
	timer.wait_time = float(every) if every.is_valid_int() else 60.0
	timer.timeout.connect(func(): _pulse(url, token, "heartbeat"))
	add_child(timer)
	_pulse(url, token, "start")
	timer.start()


func _pulse(url: String, token: String, path: String) -> void:
	var http := HTTPRequest.new()
	add_child(http)
	http.request_completed.connect(func(_r, _code, _h, _b): http.queue_free())
	http.request(url + "/" + path, ["Content-Type: application/json"], HTTPClient.METHOD_POST,
		JSON.stringify({"token": token}))
