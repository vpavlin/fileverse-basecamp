#!/usr/bin/env python3
"""collab3.py OUT_DIR : three Basecamps (t :92, u :93, v :94) edit one shared document.

t makes a new document and shares it; u and v join it by id; then rounds of typing:
each person alone, then all three at once in different places, then all three in the
same paragraph. Every screen is captured twice a second (OUT/frames/<who>/). At the end
each person's copy (the draft file, which follows the shared text) is compared.
"""
import glob, json, os, subprocess, sys, threading, time

OUT = sys.argv[1]
SESS = "/home/vpavlin/fileverse/sessions/bc-{}/home/.local/share/Logos/LogosBasecamp/module_data/fileverse_core"
WHO = {"t": ":92", "u": ":93", "v": ":94"}
os.makedirs(OUT, exist_ok=True)
log = open(os.path.join(OUT, "log.txt"), "a")

def say(*a):
    m = time.strftime("%H:%M:%S ") + " ".join(str(x) for x in a)
    print(m, flush=True); log.write(m + "\n"); log.flush()

def xdo(who, *a):
    subprocess.run(["xdotool", *a], env=dict(os.environ, DISPLAY=WHO[who]), check=False)

def click(who, x, y): xdo(who, "mousemove", str(x), str(y), "click", "1"); time.sleep(0.4)
def key(who, *ks):
    for k in ks: xdo(who, "key", "--delay", "60", k)
def typ(who, text, delay=70): xdo(who, "type", "--delay", str(delay), "--", text)
def shot(who, name): subprocess.run(["scrot", "-o", os.path.join(OUT, f"{name}-{who}.png")], env=dict(os.environ, DISPLAY=WHO[who]))

def drafts(who):
    return glob.glob(os.path.join(SESS.format(who), "*", "drafts", "*.json"))
def newest(who):
    ds = drafts(who)
    return max(ds, key=os.path.getmtime) if ds else None
def md_of(path):
    try: return json.load(open(path))["markdown"]
    except Exception as e: return f"<{e}>"
def draft_for_note(who, note):
    for p in drafts(who):
        try:
            d = json.load(open(p))
            if (d.get("reflection") or {}).get("id") == note: return p
        except Exception: pass

# ── recording: a frame per screen every 0.5 s ──
stop = threading.Event()
def record(who):
    d = os.path.join(OUT, "frames", who); os.makedirs(d, exist_ok=True)
    n = 0
    while not stop.is_set():
        subprocess.run(["scrot", "-o", os.path.join(d, f"{n:05d}.png")], env=dict(os.environ, DISPLAY=WHO[who]))
        n += 1; time.sleep(1.0)
recs = [threading.Thread(target=record, args=(w,), daemon=True) for w in WHO]

def open_local(who):
    click(who, 44, 177); time.sleep(8)        # Fileverse in the sidebar (give it time to load)
    click(who, 160, 205); time.sleep(2)       # Local
    click(who, 160, 205); time.sleep(1)

def main():
    for r in recs: r.start()
    # t: new document, share
    open_local("t")
    click("t", 228, 283); time.sleep(4)       # New document
    click("t", 800, 300); key("t", "ctrl+a"); typ("t", "Three of us"); key("t", "Return")
    typ("t", "Written by t first."); time.sleep(3)
    click("t", 1174, 72); time.sleep(6)       # Share…
    note = (json.load(open(newest("t"))).get("reflection") or {}).get("id")
    say("shared note", note)
    if not note: say("FAIL: no note id"); return
    # u and v join (again if the first try did not take)
    def join(who):
        open_local(who)
        click(who, 228, 317); time.sleep(3)   # Join a shared document…
        click(who, 228, 362); typ(who, note, 10); time.sleep(0.5)
        click(who, 228, 401)                  # Join
    for who in ("u", "v"): join(who)
    t0 = time.time(); retried = False
    while time.time() - t0 < 120:
        ok = all(draft_for_note(w, note) and "Written by t first." in md_of(draft_for_note(w, note)) for w in ("u", "v"))
        if ok: break
        if not retried and time.time() - t0 > 30:
            for w in ("u", "v"):
                if not draft_for_note(w, note): say("retrying the join on", w); join(w)
            retried = True
        time.sleep(2)
    say("joined; text on u and v after", round(time.time() - t0), "s:", ok)
    for w in WHO: shot(w, "1-joined")
    paths = {w: (newest("t") if w == "t" else draft_for_note(w, note)) for w in WHO}

    # round 1: one after another, each a new line at the end
    for w in ("u", "v", "t"):
        click(w, 800, 600); key(w, "ctrl+End", "Return"); typ(w, f"{w} adds a line on its own."); time.sleep(4)
    wait_same(paths, "round 1 (in turn)")
    for w in WHO: shot(w, "2-in-turn")

    # round 2: all three at once, each at the end of a different line
    lines = {"t": 245, "u": 276, "v": 307}
    th = []
    for w in WHO:
        click(w, 1100, lines[w]); key(w, "End")
    for w in WHO:
        th.append(threading.Thread(target=typ, args=(w, SAME[w], 150)))
    for x in th: x.start()
    for x in th: x.join()
    wait_same(paths, "round 2 (same time, different lines)")
    for w in WHO:
        missing = [o for o in WHO if SAME[o].strip() not in md_of(paths[w])]
        say(f"round 2 on {w}: " + ("every sentence intact" if not missing else "garbled or missing: " + ", ".join(missing)))
    for w in WHO: shot(w, "3-same-time")

    # round 3: all three at once at the end of the document
    for w in WHO:
        click(w, 800, 600); key(w, "ctrl+End")
    th = [threading.Thread(target=typ, args=(w, f" {w}{w}{w}", 120)) for w in WHO]
    for x in th: x.start()
    for x in th: x.join()
    wait_same(paths, "round 3 (same time, same place)")
    for w in WHO: shot(w, "4-same-place")
    stop.set()
    for w in WHO:
        say(f"--- {w}:\n" + md_of(paths[w]))

SAME = {w: f" [{w} typing at the same time in its own line]" for w in WHO}

def wait_same(paths, what, limit=120):
    t0 = time.time()
    while time.time() - t0 < limit:
        texts = {w: md_of(p) for w, p in paths.items()}
        if len(set(texts.values())) == 1:
            say(f"{what}: all three identical after {round(time.time() - t0)} s")
            return True
        time.sleep(2)
    say(f"{what}: NOT identical after {limit} s")
    for w, t in texts.items(): say(f"  {w}: {t!r}")
    return False

try:
    main()
finally:
    stop.set()
