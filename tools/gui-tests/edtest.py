#!/usr/bin/env python3
"""edtest.py DISPLAY DRAFT_JSON CASES_MODULE OUT_DIR : drive Fileverse's editor with xdotool.

Each case: reset the editor to an empty paragraph, run its steps, wait for the save, read
the draft's Markdown back from disk, screenshot, and compare with what the case expects.
"""
import json, os, subprocess, sys, time, importlib.util

DISPLAY, DRAFT, CASES, OUT = sys.argv[1:5]
ENV = dict(os.environ, DISPLAY=DISPLAY)
os.makedirs(OUT, exist_ok=True)

# toolbar (visual editor), y = 121
TOOL = {"Text": 413, "H1": 450, "H2": 483, "H3": 517, "B": 559, "I": 583, "S": 610, "Code": 641,
        "Link": 680, "List": 734, "OList": 781, "Task": 837, "Quote": 896, "CodeBlock": 957,
        "Image": 1034, "Columns": 1108, "Undo": 1174, "Redo": 1203, "Visual": 1256, "Markdown": 1323}
EDITOR = (800, 400)

def xdo(*a):
    subprocess.run(["xdotool", *a], env=ENV, check=False)

def shot(name):
    subprocess.run(["scrot", "-o", os.path.join(OUT, name + ".png")], env=ENV, check=False)

def md():
    try:
        return json.load(open(DRAFT))["markdown"]
    except Exception as e:
        return f"<unreadable: {e}>"

def step(s):
    op = s[0]
    if op == "type":
        xdo("type", "--delay", "45", "--", s[1])
    elif op == "text":   # several lines: Return between them (xdotool type drops newlines here)
        for k, line in enumerate(s[1].split("\n")):
            if k:
                xdo("key", "Return")
            if line:
                xdo("type", "--delay", "45", "--", line)
    elif op == "key":
        for k in s[1].split():
            xdo("key", "--delay", "80", k)
    elif op == "tool":
        xdo("mousemove", str(TOOL[s[1]]), "121", "click", "1")
        time.sleep(0.6)
    elif op == "click":
        xdo("mousemove", str(s[1]), str(s[2]), "click", "1")
        time.sleep(0.4)
    elif op == "wait":
        time.sleep(s[1])
    elif op == "shot":
        shot(s[1])

def reset(markdown_mode=False):
    xdo("mousemove", str(TOOL["Markdown" if markdown_mode else "Visual"]), "121", "click", "1")
    time.sleep(0.8)
    xdo("mousemove", *map(str, EDITOR), "click", "1")
    time.sleep(0.2)
    xdo("key", "ctrl+a"); xdo("key", "Delete")
    time.sleep(0.3)
    if not markdown_mode:
        xdo("mousemove", str(TOOL["Text"]), "121", "click", "1")
        time.sleep(0.4)
        xdo("mousemove", *map(str, EDITOR), "click", "1")
        time.sleep(0.2)

spec = importlib.util.spec_from_file_location("cases", CASES)
mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod)
only = set(sys.argv[5:])
results = []
for c in mod.CASES:
    if only and c["name"] not in only:
        continue
    reset(c.get("markdown", False))
    for s in c["steps"]:
        step(s)
    time.sleep(c.get("settle", 2.2))
    got = md()
    shot(c["name"])
    want = c.get("want")
    ok = None
    if want is not None:
        norm = lambda t: t.replace("\r", "").strip(" \n")
        ok = (norm(got) == norm(want)) if not c.get("contains") else all(w in got for w in (want if isinstance(want, list) else [want]))
    results.append({"name": c["name"], "ok": ok, "got": got, "want": want, "note": c.get("note", "")})
    print(("PASS" if ok else "FAIL" if ok is False else "LOOK"), c["name"], repr(got)[:160], flush=True)
json.dump(results, open(os.path.join(OUT, "results.json"), "w"), indent=1)
