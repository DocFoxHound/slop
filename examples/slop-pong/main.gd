# Slop Pong: the test game for SLAPorSLOP. You against the computer, first to 7. It keeps your wins
# and best rally in SLAPorSLOP's save folder, loads mods from SLAPorSLOP (colours, speed, and code that
# changes the game), asks the SLAPorSLOP app for a test server's mods, and counts players with Pulse.
extends Node2D

const SIZE := Vector2(800, 600)
const PADDLE := Vector2(14, 90)
const BALL := 12.0
const WIN_SCORE := 7
const SAVE := "pong.json"
## The mod the pretend online server needs (its SLAPorSLOP address).
const SERVER_MODS := [{"slug": "slop-pong-neon"}]

var paddle_speed := 420.0
var ball_speed := 360.0
var ai_speed := 300.0
var colors := {"background": Color("#14141c"), "paddle": Color.WHITE, "ball": Color.WHITE}

var left_y := SIZE.y / 2
var right_y := SIZE.y / 2
var ball_pos := SIZE / 2
var ball_vel := Vector2.ZERO
var score := [0, 0]
var rally := 0
var playing := false
var stats := {"wins": 0, "losses": 0, "best_rally": 0}

var _menu: VBoxContainer
var _status: Label
var _score_label: Label


func _ready() -> void:
	# A save is data from disk: take only the numbers expected.
	var saved := Slop.load_json(SAVE)
	for key in stats.keys():
		if saved.get(key) is float:
			stats[key] = maxi(int(saved[key]), 0)
	_build_menu()
	_apply_mods()
	_show_menu("")


# --- Mods -------------------------------------------------------------------------------------

## Starts from the defaults, then lets each mod change them. A mod.json may set "colors"
## (background, paddle, ball as #rrggbb), "ball_speed" and "paddle_speed" (0.5 to 2 times), and
## name a "script" whose setup(game) and process(game, delta) are called.
func _apply_mods() -> void:
	paddle_speed = 420.0
	ball_speed = 360.0
	colors = {"background": Color("#14141c"), "paddle": Color.WHITE, "ball": Color.WHITE}
	for mod in Slop.load_mods():
		var c = mod.get("colors")
		if c is Dictionary:
			for key in colors.keys():
				if c.get(key) is String and Color.html_is_valid(c[key]):
					colors[key] = Color.html(c[key])
		if mod.get("ball_speed") is float:
			ball_speed *= clampf(mod["ball_speed"], 0.5, 2.0)
		if mod.get("paddle_speed") is float:
			paddle_speed *= clampf(mod["paddle_speed"], 0.5, 2.0)
		var code = mod.get("instance")
		if code != null and code.has_method("setup"):
			code.setup(self)
	queue_redraw()


func _mod_names() -> String:
	var names := PackedStringArray()
	for mod in Slop.mods:
		names.append(mod["name"])
	return ", ".join(names) if names.size() > 0 else "none"


# --- Menu ---------------------------------------------------------------------------------------

func _build_menu() -> void:
	var layer := CanvasLayer.new()
	add_child(layer)
	_score_label = Label.new()
	_score_label.position = Vector2(0, 16)
	_score_label.size = Vector2(SIZE.x, 40)
	_score_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_score_label.add_theme_font_size_override("font_size", 32)
	layer.add_child(_score_label)

	_menu = VBoxContainer.new()
	_menu.position = Vector2(200, 170)
	_menu.size = Vector2(400, 300)
	_menu.add_theme_constant_override("separation", 12)
	layer.add_child(_menu)
	var title := Label.new()
	title.text = "SLOP PONG"
	title.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	title.add_theme_font_size_override("font_size", 44)
	_menu.add_child(title)
	_menu.add_child(_button("Play", _start_match))
	_menu.add_child(_button("Play online (test server)", _play_online))
	_menu.add_child(_button("Quit", func(): get_tree().quit()))
	_status = Label.new()
	_status.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_status.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	_menu.add_child(_status)


func _button(text: String, pressed: Callable) -> Button:
	var b := Button.new()
	b.text = text
	b.pressed.connect(pressed)
	return b


func _show_menu(message: String) -> void:
	playing = false
	_menu.visible = true
	var lines := PackedStringArray()
	if not message.is_empty():
		lines.append(message)
	lines.append("Wins %d · losses %d · best rally %d" % [stats.wins, stats.losses, stats.best_rally])
	lines.append("Mods: " + _mod_names())
	lines.append("Saves: " + ProjectSettings.globalize_path(Slop.save_dir()))
	_status.text = "\n".join(lines)
	_menu.get_child(1).grab_focus()


## The pretend server needs the Neon mod: ask the SLAPorSLOP app for it, then play.
func _play_online() -> void:
	_status.text = "The test server needs the Neon mod. Asking the SLAPorSLOP app; answer it there…"
	var answer: Dictionary = await Slop.request_mods(SERVER_MODS, "Slop Pong test server")
	match answer.get("status"):
		"done":
			_apply_mods()
			_start_match()
		"declined":
			_show_menu("You said no to the server's mods, so you can't join it.")
		"no_link":
			_show_menu("Start Slop Pong from the SLAPorSLOP app to get the server's mods.")
		_:
			_show_menu("Couldn't get the server's mods: %s" % answer.get("message", answer.get("status")))


# --- The game -----------------------------------------------------------------------------------

func _start_match() -> void:
	score = [0, 0]
	_menu.visible = false
	playing = true
	_serve(1 if randf() < 0.5 else -1)


func _serve(direction: int) -> void:
	ball_pos = SIZE / 2
	rally = 0
	var angle := randf_range(-0.6, 0.6)
	ball_vel = Vector2(direction, 0).rotated(angle) * ball_speed


func _process(delta: float) -> void:
	for mod in Slop.mods:
		var code = mod.get("instance")
		if code != null and code.has_method("process"):
			code.process(self, delta)
	if playing:
		_step(delta)
	_score_label.text = "%d   %d" % score
	queue_redraw()


func _step(delta: float) -> void:
	var move := Input.get_axis("ui_up", "ui_down")
	if Input.is_key_pressed(KEY_W):
		move = -1.0
	elif Input.is_key_pressed(KEY_S):
		move = 1.0
	left_y = clampf(left_y + move * paddle_speed * delta, PADDLE.y / 2, SIZE.y - PADDLE.y / 2)
	right_y = move_toward(right_y, ball_pos.y, ai_speed * delta)
	right_y = clampf(right_y, PADDLE.y / 2, SIZE.y - PADDLE.y / 2)

	ball_pos += ball_vel * delta
	if ball_pos.y < BALL / 2 or ball_pos.y > SIZE.y - BALL / 2:
		ball_vel.y = -ball_vel.y
		ball_pos.y = clampf(ball_pos.y, BALL / 2, SIZE.y - BALL / 2)
	if ball_vel.x < 0 and _hits(Vector2(30, left_y)):
		_bounce(left_y)
	elif ball_vel.x > 0 and _hits(Vector2(SIZE.x - 30, right_y)):
		_bounce(right_y)
	if ball_pos.x < 0:
		_point(1)
	elif ball_pos.x > SIZE.x:
		_point(0)


func _hits(paddle: Vector2) -> bool:
	return absf(ball_pos.x - paddle.x) < (PADDLE.x + BALL) / 2 \
		and absf(ball_pos.y - paddle.y) < (PADDLE.y + BALL) / 2


## Off a paddle: faster each time, angled by where it hit.
func _bounce(paddle_y: float) -> void:
	rally += 1
	var offset := (ball_pos.y - paddle_y) / (PADDLE.y / 2)
	var speed := minf(ball_vel.length() * 1.05, ball_speed * 2.5)
	ball_vel = Vector2(-signf(ball_vel.x), offset * 0.7).normalized() * speed


func _point(winner: int) -> void:
	score[winner] += 1
	stats.best_rally = maxi(stats.best_rally, rally)
	if score[winner] >= WIN_SCORE:
		if winner == 0:
			stats.wins += 1
		else:
			stats.losses += 1
		Slop.save_json(SAVE, stats)
		_show_menu("You won!" if winner == 0 else "The computer won.")
	else:
		_serve(-1 if winner == 0 else 1)


func _draw() -> void:
	draw_rect(Rect2(Vector2.ZERO, SIZE), colors.background)
	for y in range(0, int(SIZE.y), 30):
		draw_rect(Rect2(SIZE.x / 2 - 2, y, 4, 15), colors.paddle * Color(1, 1, 1, 0.3))
	draw_rect(Rect2(Vector2(30, left_y) - PADDLE / 2, PADDLE), colors.paddle)
	draw_rect(Rect2(Vector2(SIZE.x - 30, right_y) - PADDLE / 2, PADDLE), colors.paddle)
	if playing:
		draw_rect(Rect2(ball_pos - Vector2(BALL, BALL) / 2, Vector2(BALL, BALL)), colors.ball)
