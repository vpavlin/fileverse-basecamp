CASES = [
  {"name": "typed_code_then_plain", "steps": [("type", "path `~/.local/share/x` here"), ("shot", "code_typed")], "want": "path `~/.local/share/x` here"},
  {"name": "toolbar_code", "steps": [("type", "run make"), ("key", "ctrl+shift+Left"), ("tool", "Code"), ("key", "End"), ("type", " now"), ("shot", "code_tool")], "want": "run `make` now"},
]
