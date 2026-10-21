# Rainbow ball: a mod with code. Slop Pong calls setup(game) once and process(game, delta)
# every frame. It only changes the game's own colours; it reads no files and goes nowhere.
extends RefCounted

var hue := 0.0


func setup(game) -> void:
	game.colors["paddle"] = Color.from_hsv(0.0, 0.0, 0.9)


func process(game, delta: float) -> void:
	hue = fmod(hue + delta * 0.25, 1.0)
	game.colors["ball"] = Color.from_hsv(hue, 0.8, 1.0)
