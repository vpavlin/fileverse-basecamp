# Undo / redo in steps (0.14.4)
CASES = [
  {"name": "undo_last_word", "steps": [("type", "one two three"), ("key", "ctrl+z")], "want": "one two"},
  {"name": "undo_two_words", "steps": [("type", "one two three"), ("key", "ctrl+z ctrl+z")], "want": "one"},
  {"name": "redo_ctrl_y", "steps": [("type", "one two three"), ("key", "ctrl+z ctrl+y")], "want": "one two three"},
  {"name": "redo_ctrl_shift_z", "steps": [("type", "one two"), ("key", "ctrl+z ctrl+shift+z")], "want": "one two"},
  {"name": "undo_after_pause", "steps": [("type", "abc"), ("wait", 1.2), ("type", "def"), ("key", "ctrl+z")], "want": "abc"},
  {"name": "undo_bold", "steps": [("type", "x"), ("key", "shift+Home ctrl+b"), ("wait", 0.5), ("key", "ctrl+z")], "want": "x"},
  {"name": "undo_list_style", "steps": [("type", "item"), ("tool", "List"), ("wait", 0.5), ("key", "ctrl+z")], "want": "item"},
  {"name": "undo_toolbar", "steps": [("type", "keep drop"), ("tool", "Undo")], "want": "keep"},
  {"name": "undo_enter", "steps": [("type", "a"), ("key", "Return"), ("type", "b"), ("key", "ctrl+z ctrl+z")], "want": "a"},
  {"name": "md_undo_word", "markdown": True, "steps": [("type", "hello world"), ("key", "ctrl+z")], "want": "hello"},
  {"name": "md_redo", "markdown": True, "steps": [("type", "hello world"), ("key", "ctrl+z ctrl+y")], "want": "hello world"},
]
