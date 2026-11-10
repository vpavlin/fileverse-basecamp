# Links, Markdown mode, switching, columns, tables, page breaks, diagrams, focus.
RICH = ("# Plan\n\nWe go **self-sovereign** with *care* and ~~servers~~ `code`.\n\n## Steps\n\n- one\n    - nested\n- two\n\n"
        "1. first\n2. second\n\n- [ ] open\n- [x] done\n\n> a quote\n\n```js\nlet x = 1;\n```\n\n"
        "| a | b |\n|---|---|\n| 1 | 2 |\n\n[Logos](https://logos.co) and https://example.org\n\n===\n\n# Slide two\n\n"
        "```mermaid\nflowchart TB\n  a --> b\n```\n")
CASES = [
  {"name": "link_ctrl_k", "steps": [("type", "site"), ("key", "shift+Home ctrl+k"), ("wait", 0.6), ("type", "https://logos.co"), ("key", "Return")],
   "want": "[site](https://logos.co)"},
  {"name": "link_tool", "steps": [("type", "docs"), ("key", "shift+Home"), ("tool", "Link"), ("wait", 0.6), ("type", "https://docs.logos.co"), ("key", "Return")],
   "want": "[docs](https://docs.logos.co)"},
  {"name": "bare_url_typed", "steps": [("type", "see https://logos.co now")], "want": "see https://logos.co now"},
  {"name": "md_mode_rich", "markdown": True, "steps": [("text", RICH)], "want": RICH, "settle": 3},
  {"name": "md_to_visual_and_back", "markdown": True,
   "steps": [("text", RICH), ("wait", 2.5), ("tool", "Visual"), ("wait", 2.5), ("shot", "rich_visual"), ("tool", "Markdown"), ("wait", 2)],
   "want": RICH, "settle": 3},
  {"name": "visual_edit_keeps_rest", "markdown": True,
   "steps": [("text", RICH), ("wait", 2.5), ("tool", "Visual"), ("wait", 2.5), ("click", 700, 190), ("key", "End"), ("type", " now"), ("wait", 2.5), ("tool", "Markdown"), ("wait", 2)],
   "want": RICH.replace("# Plan", "# Plan now"), "settle": 3, "note": "one word added to the heading in the visual editor; nothing else may change"},
  {"name": "columns_tool", "steps": [("type", "left text"), ("tool", "Columns"), ("wait", 1.5)], "want": None, "note": "what Columns inserts"},
  {"name": "page_break_md", "markdown": True, "steps": [("text", "# A\n\ntext\n===\n# B\n")], "want": "# A\n\ntext\n===\n# B"},
  {"name": "title_tab", "steps": [("click", 600, 73), ("key", "ctrl+a"), ("type", "My title"), ("key", "Tab"), ("type", "body")], "want": None,
   "note": "Tab from the title field: into the editor?"},
]
