import json, os, subprocess, time, glob
NOTE = "9e1c4f1d94146a4799f7da2d84813459d11a918d64473b13da27305307f19eef"
W = {"t": ":92", "u": ":93"}
def x(w, *a): subprocess.run(["xdotool", *a], env=dict(os.environ, DISPLAY=W[w]))
def click(w, a, b): x(w, "mousemove", str(a), str(b), "click", "1"); time.sleep(0.5)
def draft(w):
    for p in glob.glob(f"/home/vpavlin/fileverse/sessions/bc-{w}/home/.local/share/Logos/LogosBasecamp/module_data/fileverse_core/*/drafts/*.json"):
        d = json.load(open(p))
        if (d.get("reflection") or {}).get("id") == NOTE: return p
def md(w): return json.load(open(draft(w)))["markdown"]
for w in W:
    click(w, 44, 177); time.sleep(8); click(w, 160, 205); time.sleep(2)
    click(w, 228, 317); time.sleep(2); click(w, 228, 362); x(w, "type", "--delay", "10", NOTE); click(w, 228, 401); time.sleep(6)
time.sleep(10)
print("start t:", repr(md("t")[-60:]))
click("t", 1100, 239); x("t", "key", "End"); x("t", "type", "--delay", "80", " TWORD"); time.sleep(4)
click("u", 1100, 332); x("u", "key", "End"); x("u", "type", "--delay", "80", " UWORD"); time.sleep(15)
print("both have both:", all("TWORD" in md(w) and "UWORD" in md(w) for w in W))
click("t", 1100, 239); x("t", "key", "End", "ctrl+z"); time.sleep(15)
for w in W: print(w, "TWORD" in md(w), "UWORD" in md(w))
subprocess.run(["scrot", "-o", "/tmp/undo2-t.png"], env=dict(os.environ, DISPLAY=":92"))
