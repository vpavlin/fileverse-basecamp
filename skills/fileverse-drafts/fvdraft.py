#!/usr/bin/env python3
"""fvdraft: read and write Fileverse drafts kept by the Fileverse app in Logos Basecamp.

Drafts are the documents you edit in Basecamp's Fileverse app before publishing. Each is
one JSON file in the fileverse_core module's data directory; Basecamp lists that
directory every few seconds, so a draft written here shows up there, and an open draft
that changed on disk is reloaded (or, if you were typing, Basecamp asks which version to
keep).

Commands (all print JSON or Markdown on stdout, errors on stderr, exit 1 on error):

  fvdraft.py where                      the drafts directories found (first = used)
  fvdraft.py list                       id, title, updatedAt, published? of each draft
  fvdraft.py show ID                    the draft's Markdown
  fvdraft.py info ID                    the draft's fields (never its secret)
  fvdraft.py new --title T [--file F]   create a draft (Markdown from F, or stdin with -)
  fvdraft.py save ID --file F [--title T] [--base UPDATED_AT]
                                        replace a draft's Markdown (and title); with
                                        --base, refuse if it changed since you read it
  fvdraft.py add-image ID PATH [--alt TEXT]
                                        keep a picture with the draft; prints the
                                        Markdown line to put where it should appear
  fvdraft.py lint (ID | --file F)       check Markdown for things that will not show well

Where: FV_DRAFTS (a drafts directory) wins; else FV_BASECAMP_USER_DIR (Basecamp's
--user-dir; several separated by ':'), the --user-dir of a running Basecamp (remembered in
~/.cache/fvdraft/user-dirs for when it is closed), and Basecamp's default data
directories. Python 3 standard library only.
"""

import argparse
import base64
import glob
import hashlib
import json
import os
import re
import secrets
import sys
import time

MAX_IMAGE = 25 * 1024 * 1024
ID_RE = re.compile(r"^[A-Za-z0-9]+$")


def die(msg):
    print(f"fvdraft: {msg}", file=sys.stderr)
    sys.exit(1)


REMEMBERED = os.path.join(os.path.expanduser("~"), ".cache", "fvdraft", "user-dirs")


def user_dirs_running():
    """The --user-dir of running Basecamp processes (Linux /proc; else ps)."""
    out = []

    def from_argv(argv, cwd=None):
        for i, a in enumerate(argv):
            v = None
            if a == "--user-dir" and i + 1 < len(argv):
                v = argv[i + 1]
            elif a.startswith("--user-dir="):
                v = a.split("=", 1)[1]
            if v:
                v = os.path.expanduser(v)
                if not os.path.isabs(v) and cwd:
                    v = os.path.join(cwd, v)
                out.append(os.path.normpath(v))

    if os.path.isdir("/proc"):
        for pid in os.listdir("/proc"):
            if not pid.isdigit():
                continue
            try:
                with open(f"/proc/{pid}/cmdline", "rb") as f:
                    argv = [x.decode(errors="replace") for x in f.read().split(b"\0") if x]
                if not any(a.startswith("--user-dir") for a in argv):
                    continue
                try:
                    cwd = os.readlink(f"/proc/{pid}/cwd")
                except OSError:
                    cwd = None
                from_argv(argv, cwd)
            except OSError:
                continue
    else:
        try:
            import subprocess
            ps = subprocess.run(["ps", "-axo", "command"], capture_output=True, text=True, timeout=5).stdout
            for line in ps.splitlines():
                if "--user-dir" in line:
                    from_argv(line.split())
        except Exception:
            pass
    return out


def remembered():
    try:
        with open(REMEMBERED, encoding="utf-8") as f:
            return [l.strip() for l in f if l.strip()]
    except OSError:
        return []


def remember(dirs):
    known = remembered()
    new = [d for d in dirs if d not in known]
    if not new:
        return
    try:
        os.makedirs(os.path.dirname(REMEMBERED), exist_ok=True)
        with open(REMEMBERED, "a", encoding="utf-8") as f:
            for d in new:
                f.write(d + "\n")
    except OSError:
        pass


def candidates():
    """Drafts directories, the most recently changed first. Basecamp keeps module data
    under its user directory: `--user-dir DIR` (then DIR/module_data/...) or, by
    default, ~/.local/share/Logos/LogosBasecamp (Linux) / ~/Library/Application
    Support/Logos/LogosBasecamp (macOS)."""
    home = os.path.expanduser("~")
    roots = []
    env = os.environ.get("FV_BASECAMP_USER_DIR")
    if env:
        roots += [os.path.expanduser(r) for r in env.split(os.pathsep) if r]
    running = user_dirs_running()
    remember([r for r in running if glob.glob(os.path.join(r, "module_data/fileverse_core/*/drafts"))])
    roots += running + remembered()
    roots += [
        os.path.join(home, ".local/share/Logos/LogosBasecamp"),
        os.path.join(home, "Library/Application Support/Logos/LogosBasecamp"),
    ]
    found = []
    for r in roots:
        for d in glob.glob(os.path.join(r, "module_data/fileverse_core/*/drafts")):
            d = os.path.normpath(d)
            if os.path.isdir(d) and d not in found:
                found.append(d)

    def last_change(d):
        times = [os.path.getmtime(f) for f in glob.glob(os.path.join(d, "*.json"))]
        return max(times + [os.path.getmtime(d)])

    return sorted(found, key=last_change, reverse=True)


def drafts_dir():
    env = os.environ.get("FV_DRAFTS")
    if env:
        if not os.path.isdir(env):
            die(f"FV_DRAFTS={env} is not a directory")
        return env
    found = candidates()
    if not found:
        die("no Fileverse drafts directory found: open the Fileverse app in Basecamp once (while it runs, a --user-dir is found), "
            "or set FV_BASECAMP_USER_DIR to Basecamp's --user-dir, or FV_DRAFTS to the drafts directory")
    return found[0]


def draft_path(d, draft_id):
    if not ID_RE.match(draft_id or ""):
        die(f"bad draft id {draft_id!r}")
    return os.path.join(d, f"{draft_id}.json")


def load(d, draft_id):
    p = draft_path(d, draft_id)
    try:
        with open(p, encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        die(f"no draft {draft_id}")


def write(d, draft):
    """Atomically, readable by you only. The temporary file sits in the draft's own
    subdirectory: Basecamp lists every JSON file in the drafts directory itself."""
    sub = os.path.join(d, draft["id"])
    os.makedirs(sub, exist_ok=True)
    tmp = os.path.join(sub, ".write.tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(draft, f, ensure_ascii=False, separators=(",", ":"))
    os.chmod(tmp, 0o600)
    os.replace(tmp, draft_path(d, draft["id"]))


def now_ms():
    return int(time.time() * 1000)


def read_markdown(path):
    if path in (None, "-"):
        return sys.stdin.read()
    with open(path, encoding="utf-8") as f:
        return f.read()


def public(draft):
    out = {k: v for k, v in draft.items() if k != "secret"}
    return out


# ── commands ────────────────────────────────────────────────────────────────


def cmd_where(_a):
    print(json.dumps({
        "FV_DRAFTS": os.environ.get("FV_DRAFTS"),
        "FV_BASECAMP_USER_DIR": os.environ.get("FV_BASECAMP_USER_DIR"),
        "running_user_dirs": user_dirs_running(),
        "remembered_user_dirs": remembered(),
        "found": candidates(),
    }, indent=2))


def cmd_list(_a):
    d = drafts_dir()
    out = []
    for f in glob.glob(os.path.join(d, "*.json")):
        try:
            with open(f, encoding="utf-8") as fh:
                x = json.load(fh)
        except (OSError, ValueError):
            continue
        out.append({"id": x.get("id"), "title": x.get("title"), "updatedAt": x.get("updatedAt"),
                    "published": bool(x.get("published")), "sourceUrl": x.get("sourceUrl") or None})
    out.sort(key=lambda x: x.get("updatedAt") or 0, reverse=True)
    print(json.dumps(out, indent=2, ensure_ascii=False))


def cmd_show(a):
    sys.stdout.write(load(drafts_dir(), a.id).get("markdown", ""))


def cmd_info(a):
    print(json.dumps(public(load(drafts_dir(), a.id)), indent=2, ensure_ascii=False))


def cmd_new(a):
    d = drafts_dir()
    md = read_markdown(a.file) if a.file else ""
    t = now_ms()
    draft = {
        "id": secrets.token_hex(8),
        "title": a.title.strip() or "Untitled",
        "markdown": md,
        "sourceUrl": "",
        "createdAt": t,
        "updatedAt": t,
        # the key publishing uses for this draft (as the app makes it); never print it
        "secret": base64.b64encode(secrets.token_bytes(32)).decode(),
    }
    write(d, draft)
    print(json.dumps({"id": draft["id"], "updatedAt": t, "dir": d}))


def cmd_save(a):
    d = drafts_dir()
    draft = load(d, a.id)
    if a.base is not None and str(draft.get("updatedAt")) != str(a.base):
        die(f"draft {a.id} changed since you read it (updatedAt {draft.get('updatedAt')}, you had {a.base}); "
            "read it again and merge your change")
    draft["markdown"] = read_markdown(a.file)
    if a.title is not None:
        draft["title"] = a.title.strip() or "Untitled"
    draft["updatedAt"] = max(now_ms(), int(draft.get("updatedAt") or 0) + 1)
    write(d, draft)
    print(json.dumps({"id": a.id, "updatedAt": draft["updatedAt"]}))


def sniff(b):
    if b.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png", "png"
    if b.startswith(b"\xff\xd8\xff"):
        return "image/jpeg", "jpg"
    if b[:6] in (b"GIF87a", b"GIF89a"):
        return "image/gif", "gif"
    if b[:4] == b"RIFF" and b[8:12] == b"WEBP":
        return "image/webp", "webp"
    head = b[:512].lstrip().lower()
    if head.startswith(b"<svg") or (head.startswith(b"<?xml") and b"<svg" in head):
        return "image/svg+xml", "svg"
    return None, None


def cmd_add_image(a):
    d = drafts_dir()
    load(d, a.id)  # the draft must exist
    size = os.path.getsize(a.path)
    if size > MAX_IMAGE:
        die("that picture is larger than 25 MB")
    with open(a.path, "rb") as f:
        b = f.read()
    mime, ext = sniff(b)
    if not mime:
        die("not a picture the app shows (PNG, JPEG, GIF, WebP or SVG)")
    name = f"{hashlib.sha256(b).hexdigest()[:24]}.{ext}"
    img = os.path.join(d, a.id, "img")
    os.makedirs(img, exist_ok=True)
    with open(os.path.join(img, name), "wb") as f:
        f.write(b)
    ref = {"mime": mime, "width": "100%", "local": f"{a.id}/{name}"}
    url = "fvimg:" + base64.urlsafe_b64encode(json.dumps(ref, separators=(",", ":")).encode()).decode().rstrip("=")
    alt = (a.alt if a.alt is not None else os.path.splitext(os.path.basename(a.path))[0]).replace("[", "").replace("]", "")
    print(f"![{alt}]({url})")


MERMAID_KINDS = ("flowchart", "graph", "sequenceDiagram", "classDiagram", "stateDiagram", "stateDiagram-v2",
                 "erDiagram", "gantt", "pie", "journey", "mindmap", "timeline", "gitGraph", "quadrantChart",
                 "requirementDiagram", "C4Context", "sankey-beta", "xychart-beta", "block-beta", "kanban")


def lint(md):
    notes = []
    lines = md.splitlines()
    fence, fence_lang, fence_at, body = False, "", 0, []
    cols_open, col_line = False, 0
    has_h1 = False
    for i, line in enumerate(lines, 1):
        t = line.strip()
        if t.startswith("```"):
            if not fence:
                fence, fence_lang, fence_at, body = True, t[3:].strip(), i, []
            else:
                fence = False
                first = next((l.strip() for l in body if l.strip() and not l.strip().startswith("%%")), "")
                kind = first.split()[0] if first.split() else ""
                if fence_lang == "mermaid" or (not fence_lang and kind in MERMAID_KINDS):
                    if not fence_lang:
                        notes.append(f"line {fence_at}: label this fence ```mermaid (older app versions and Fileverse need the label)")
                    if kind not in MERMAID_KINDS:
                        notes.append(f"line {fence_at}: diagram does not start with a known Mermaid type ({kind or 'empty'})")
                    src = "\n".join(body)
                    if kind == "sequenceDiagram":
                        if re.search(r"^\s*loop\b", src, re.M):
                            notes.append(f"line {fence_at}: `loop` in a sequence diagram: the app's renderer misplaces labels around it")
                        if re.search(r"^\s*autonumber\b", src, re.M):
                            notes.append(f"line {fence_at}: `autonumber` draws numbers over labels in the app")
                        for m in re.finditer(r"^\s*([A-Za-z0-9_]+)\s*-+>>?\s*([A-Za-z0-9_]+)\s*:", src, re.M):
                            if m.group(1) == m.group(2):
                                notes.append(f"line {fence_at}: a message from {m.group(1)} to itself is drawn badly; use a Note instead")
                                break
                    if re.search(r"^\s*direction\s+(TB|TD|LR|RL|BT)\b", src, re.M):
                        notes.append(f"line {fence_at}: `direction` inside a subgraph is ignored by the app's renderer")
            continue
        if fence:
            body.append(line)
            continue
        if t.startswith("# "):
            has_h1 = True
        if t.startswith(":::"):
            w = t[3:].strip()
            if w == "columns" or w.startswith("columns "):
                if cols_open:
                    notes.append(f"line {i}: columns cannot go inside columns")
                cols_open, col_line = True, i
            elif w == "column":
                if not cols_open:
                    notes.append(f"line {i}: `::: column` outside a `::: columns` block")
            elif w == "":
                if not cols_open:
                    notes.append(f"line {i}: `:::` closes nothing")
                cols_open = False
        if re.match(r"^=+\s*$", t) and 0 < len(t) < 3:
            notes.append(f"line {i}: a slide break needs at least three `=`")
        for m in re.finditer(r"!\[[^\]]*\]\(([^)]+)\)", line):
            url = m.group(1)
            if not url.startswith("fvimg:"):
                notes.append(f"line {i}: picture {url[:40]} is not kept with the draft; add it with `fvdraft.py add-image`")
    if fence:
        notes.append(f"line {fence_at}: code fence never closed")
    if cols_open:
        notes.append(f"line {col_line}: `::: columns` never closed with `:::`")
    if not has_h1:
        notes.append("no `# Title` line: the first slide will have no title")
    return notes


def cmd_lint(a):
    md = read_markdown(a.file) if a.file else load(drafts_dir(), a.id).get("markdown", "")
    notes = lint(md)
    print(json.dumps({"ok": not notes, "notes": notes}, indent=2, ensure_ascii=False))


def main():
    p = argparse.ArgumentParser(prog="fvdraft.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("where").set_defaults(fn=cmd_where)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    for name, fn in (("show", cmd_show), ("info", cmd_info)):
        s = sub.add_parser(name)
        s.add_argument("id")
        s.set_defaults(fn=fn)
    s = sub.add_parser("new")
    s.add_argument("--title", required=True)
    s.add_argument("--file", help="Markdown file, or - for stdin")
    s.set_defaults(fn=cmd_new)
    s = sub.add_parser("save")
    s.add_argument("id")
    s.add_argument("--file", required=True, help="Markdown file, or - for stdin")
    s.add_argument("--title")
    s.add_argument("--base", help="the updatedAt you read; refuse if the draft changed since")
    s.set_defaults(fn=cmd_save)
    s = sub.add_parser("add-image")
    s.add_argument("id")
    s.add_argument("path")
    s.add_argument("--alt")
    s.set_defaults(fn=cmd_add_image)
    s = sub.add_parser("lint")
    s.add_argument("id", nargs="?")
    s.add_argument("--file")
    s.set_defaults(fn=cmd_lint)
    a = p.parse_args()
    if a.cmd == "lint" and not (a.id or a.file):
        die("lint needs a draft id or --file")
    a.fn(a)


if __name__ == "__main__":
    main()
