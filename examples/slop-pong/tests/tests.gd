# Slop Pong's tests, headless: godot --headless --path . res://tests/tests.tscn
# Exits 0 when every check passes. Uses a scratch folder for saves, mods and Slop Link.
extends Node

var failed := 0
var scratch := OS.get_cache_dir().path_join("slop-pong-tests-%d" % Time.get_ticks_usec())


func check(ok: bool, what: String) -> void:
	if ok:
		print("ok   ", what)
	else:
		failed += 1
		printerr("FAIL ", what)


func _ready() -> void:
	DirAccess.make_dir_recursive_absolute(scratch)
	await saves()
	await mods()
	await link()
	await game()
	print("%d failed" % failed)
	remove_all(scratch)
	get_tree().quit(1 if failed > 0 else 0)


func remove_all(dir: String) -> void:
	for sub in DirAccess.get_directories_at(dir):
		remove_all(dir.path_join(sub))
	for file in DirAccess.get_files_at(dir):
		DirAccess.remove_absolute(dir.path_join(file))
	DirAccess.remove_absolute(dir)


func saves() -> void:
	OS.set_environment("SLOP_SAVE_DIR", scratch.path_join("saves"))
	DirAccess.make_dir_recursive_absolute(scratch.path_join("saves"))
	check(Slop.save_dir() == scratch.path_join("saves"), "saves go to SLOP_SAVE_DIR")
	check(Slop.save_json("pong.json", {"wins": 3}), "a save is written")
	check(Slop.load_json("pong.json").get("wins") == 3.0, "and read back")
	check(not FileAccess.file_exists(scratch.path_join("saves/pong.json.tmp")), "no half-written file left")
	var f := FileAccess.open(scratch.path_join("saves/broken.json"), FileAccess.WRITE)
	f.store_string("{not json")
	f.close()
	check(Slop.load_json("broken.json").is_empty(), "a broken save reads as empty")
	check(Slop.load_json("missing.json").is_empty(), "a missing save reads as empty")


func mods() -> void:
	# Two mods as the SLAPorSLOP app hands them over: <address>/files, listed in SLOP_MODS.
	var neon := scratch.path_join("mods/slop-pong-neon/files")
	var rainbow := scratch.path_join("mods/slop-pong-rainbow-ball/files")
	var bad := scratch.path_join("mods/broken/files")
	for d in [neon, rainbow, bad]:
		DirAccess.make_dir_recursive_absolute(d)
	var here := ProjectSettings.globalize_path("res://").path_join("../slop-pong-mods")
	for pair in [[here.path_join("neon/mod.json"), neon], [here.path_join("rainbow-ball/mod.json"), rainbow],
			[here.path_join("rainbow-ball/mod.gd"), rainbow]]:
		DirAccess.copy_absolute(pair[0], pair[1].path_join(pair[0].get_file()))
	var f := FileAccess.open(bad.path_join("mod.json"), FileAccess.WRITE)
	f.store_string("[1, 2]")
	f.close()
	OS.set_environment("SLOP_MODS", ":".join([neon, rainbow, bad]))
	var loaded := Slop.load_mods()
	var names := loaded.map(func(m): return m["name"])
	check(names == ["Neon", "Rainbow ball"], "approved mods load; a broken one is skipped (%s)" % [names])
	check(loaded.size() == 2 and loaded[1].get("instance") != null, "a mod's code is created")


func link() -> void:
	var dir := scratch.path_join("link")
	DirAccess.make_dir_recursive_absolute(dir.path_join("requests"))
	DirAccess.make_dir_recursive_absolute(dir.path_join("answers"))
	OS.set_environment("SLOP_LINK", "")
	var none: Dictionary = await Slop.request_mods([{"slug": "x"}], "test")
	check(none.get("status") == "no_link", "outside SLAPorSLOP there's no link")

	OS.set_environment("SLOP_LINK", dir)
	# Play the SLAPorSLOP app: answer the request once it shows up.
	var slop_app := func():
		for i in 40:
			await get_tree().create_timer(0.1).timeout
			var files := DirAccess.get_files_at(dir.path_join("requests"))
			for file in files:
				if file.ends_with(".json"):
					var request = JSON.parse_string(FileAccess.get_file_as_string(dir.path_join("requests").path_join(file)))
					check(request["mods"] == [{"slug": "slop-pong-neon"}] and request["for"] == "test server",
						"the request names the mods and who wants them")
					DirAccess.remove_absolute(dir.path_join("requests").path_join(file))
					var answer := FileAccess.open(dir.path_join("answers").path_join(file), FileAccess.WRITE)
					answer.store_string(JSON.stringify({"status": "done", "mods": [{"slug": "slop-pong-neon", "result": "added"}], "folders": []}))
					answer.close()
					return
	slop_app.call()
	var answer: Dictionary = await Slop.request_mods([{"slug": "slop-pong-neon"}], "test server", 10.0)
	check(answer.get("status") == "done", "the answer comes back (%s)" % [answer])


func game() -> void:
	var main: Node2D = load("res://main.tscn").instantiate()
	add_child(main)
	await get_tree().process_frame
	check(main.colors["background"] == Color("#0b0221"), "the Neon mod's colours apply")
	check(is_equal_approx(main.ball_speed, 360.0 * 1.2), "and its ball speed")
	main._start_match()
	var start: Vector2 = main.ball_pos
	for i in 10:
		await get_tree().process_frame
	check(main.ball_pos != start, "the ball moves")
	# A match ends at 7 and the result is saved.
	main.score = [6, 0]
	main._point(0)
	check(not main.playing and main.stats.wins == 4, "winning a match counts (wins %s)" % main.stats.wins)
	check(Slop.load_json("pong.json").get("wins") == 4.0, "and is saved")
	main.queue_free()
