# For SLAPorSLOP's own test suite (crates/slop-client/tests/client.rs): asks the real Slop Link for
# the test server's mod, prints the answer, and exits 0 when it's "done".
extends Node


func _ready() -> void:
	var answer: Dictionary = await Slop.request_mods([{"slug": "slop-pong-neon", "version": "1.0.0"}], "Slop Pong test server", 20.0)
	print("ANSWER ", JSON.stringify(answer))
	get_tree().quit(0 if answer.get("status") == "done" else 1)
