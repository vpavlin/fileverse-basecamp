# 0.14.4: typed emphasis (ddocs.new rules), Shift+Enter, Tab in text.
CASES = [
  {"name": "typed_bold_stars", "steps": [("type", "a **b** c")], "want": "a **b** c"},
  {"name": "typed_bold_unders", "steps": [("type", "a __b__ c")], "want": "a **b** c"},
  {"name": "typed_italic_star", "steps": [("type", "a *i* c")], "want": "a *i* c"},
  {"name": "typed_italic_under", "steps": [("type", "a _i_ c")], "want": "a *i* c"},
  {"name": "typed_strike", "steps": [("type", "a ~~s~~ c")], "want": "a ~~s~~ c"},
  {"name": "typed_code", "steps": [("type", "a `x = 1` c")], "want": "a `x = 1` c"},
  {"name": "typed_multiplication_stays", "steps": [("type", "5 * 3 * 2")], "want": "5 \\* 3 \\* 2"},
  {"name": "typed_snake_case_stays", "steps": [("type", "snake_case_name")], "want": None, "note": "no italics inside a word"},
  {"name": "typed_bold_then_plain", "steps": [("type", "**b**"), ("type", " plain")], "want": "**b** plain"},
  {"name": "shift_enter_break", "steps": [("type", "l1"), ("key", "shift+Return"), ("type", "l2")], "want": "l1\\\nl2"},
  {"name": "shift_enter_in_heading", "steps": [("type", "# H"), ("key", "shift+Return"), ("type", "two")], "want": "# H\\\ntwo", "note": "heading stays a heading"},
  {"name": "tab_in_text_indents", "steps": [("type", "a"), ("key", "Home Tab")], "want": "\u00a0\u00a0\u00a0\u00a0a".encode().decode("unicode_escape")},
  {"name": "shift_tab_takes_it_back", "steps": [("type", "a"), ("key", "Home Tab shift+Tab")], "want": "a"},
  {"name": "tab_in_list_still_nests", "steps": [("type", "- one"), ("key", "Return Tab"), ("type", "n")], "want": "- one\n    - n"},
]
