// Fileverse for Logos Basecamp: a pure-QML view over the fileverse_core module.
//
// Read dDocs/dSheets (share links, or your own files with a backup key), present
// docs as slides, keep copies on your IPFS node and in Logos Storage, and write
// Markdown drafts that are published as Fileverse-format documents. No Fileverse
// server is involved; this view only draws the JSON the core returns.
//
// Text from documents is drawn two ways:
//  - block bodies use Text.StyledText over `html` that the CORE built: it escapes
//    every character of document text and only adds <b> <i> <u> <s> <code> <br>
//    and <a href> (http/https/mailto). No <img> is ever emitted, so nothing in a
//    document makes this view load a URL. Links open only when clicked.
//  - everything else (titles, cells, CIDs, errors) is Text.PlainText.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs

import Logos.Theme
import Logos.Controls
import Logos.Icons

Item {
    id: root
    width: 1100
    height: 760

    // ── core bridge (never blocking: logos.callModuleAsync, else a deferred callModule) ──
    readonly property string coreName: "fileverse_core"
    readonly property int callTimeoutMs: 20000
    function _deliver(cb, raw) {
        if (!cb) return
        try { cb(raw === undefined || raw === null ? "" : raw) } catch (e) { console.warn("fileverse view: " + e) }
    }
    function callVia(module, method, args, cb) {
        var a = args || []
        if (typeof logos === "undefined" || logos === null) { Qt.callLater(function () { root._deliver(cb, "") }); return }
        if (typeof logos.callModuleAsync === "function") {
            try { logos.callModuleAsync(module, method, a, function (res) { root._deliver(cb, res) }, root.callTimeoutMs) }
            catch (e) { Qt.callLater(function () { root._deliver(cb, "") }) }
            return
        }
        Qt.callLater(function () {
            var raw = ""
            try { raw = logos.callModule(module, method, a) } catch (e) { raw = "" }
            root._deliver(cb, raw)
        })
    }
    function core(method, args, cb) { root.callVia(root.coreName, method, args, cb) }
    // Blocking call, for the editor's millisecond conversions: an async reply could
    // overwrite what was typed while it was on its way.
    function coreSync(method, args) {
        if (typeof logos === "undefined" || logos === null || typeof logos.callModule !== "function") return null
        try { return root.j(logos.callModule(root.coreName, method, args || [])) } catch (e) { return null }
    }
    // Results may arrive JSON-quoted once or twice by the bridge.
    function j(raw) {
        var v = raw
        for (var i = 0; i < 3 && typeof v === "string"; i++) {
            var t = v.trim()
            if (t === "") return null
            try { v = JSON.parse(t) } catch (e) { return null }
        }
        return (v && typeof v === "object") ? v : null
    }
    // A core action: toast on success, the core's sentence on failure.
    function act(method, args, okMsg, onOk) {
        root.core(method, args, function (raw) {
            var r = root.j(raw)
            if (r && r.ok) { if (okMsg) root.say(okMsg); if (onOk) onOk(r); root.refresh() }
            else root.fail(r ? r.error : ("No answer from fileverse_core (" + method + ")."))
        })
    }

    // ── state ──
    property var snap: ({ recent: [], gateways: [], portals: [], drafts: [] })
    property bool coreReachable: true
    property int missedPolls: 0
    property string section: "recent"     // recent | files | drafts
    property string mode: "home"          // home | doc | edit | portal | import
    property var docv: null               // the opened document (core JSON)
    property string docUrl: ""
    property string error: ""
    property string toast: ""
    // A question before something is removed for good: { title, text, button, run }
    property var confirmAsk: null
    function confirm(title, text, button, run) { root.confirmAsk = { title: title, text: text, button: button, run: run } }
    property int tabIndex: 0
    property int sheetIndex: 0
    property bool showDetails: false
    property bool showSettings: false
    // the networks setting is reflection_core's: read it when Settings opens (not while a
    // note is open: asking for no note would make reflection_core treat it as closed)
    onShowSettingsChanged: if (root.showSettings && !(root.mode === "edit" && root.viaReflection)) root.probeReflection()
    property var node: ({ reachable: false })
    property var storage: ({ available: false })
    property string portal: ""
    property var portalFiles: []
    property string portalError: ""
    property var portalBackup: ({})
    property var portalListedAt: null
    property var draft: null              // { id, title, markdown, preview, published }
    property bool draftDirty: false

    function say(msg) { root.toast = msg; toastTimer.restart() }
    function fail(msg) { var m = String(msg || "Something went wrong."); root.error = m.charAt(0).toUpperCase() + m.slice(1) }

    // ── jobs: every network action runs in the core; we poll until it settles ──
    property var jobs: ({})               // id -> { label }
    property var jobCallbacks: ({})
    property int jobTick: 0
    readonly property bool busy: Object.keys(root.jobs).length > 0
    function busyWith(label) { for (var k in root.jobs) if (root.jobs[k].label === label) return true; return false }
    function startJob(method, args, label, onDone, onFail) {
        if (root.busyWith(label)) return
        root.error = ""
        root.core(method, args, function (raw) {
            var r = root.j(raw)
            if (!r) { root.fail("No answer from fileverse_core. Is it installed and loaded?"); return }
            if (!r.ok) { root.fail(r.error); return }
            var js = root.jobs; js[r.jobId] = { label: label, started: Date.now() }; root.jobs = js
            var cbs = root.jobCallbacks; cbs[r.jobId] = { done: onDone, fail: onFail }; root.jobCallbacks = cbs
            root.pollJobs()
        })
    }
    property bool pollBusy: false
    function pollJobs() {
        if (root.pollBusy) return
        var ids = Object.keys(root.jobs)
        if (ids.length === 0) return
        root.pollBusy = true
        var left = ids.length
        ids.forEach(function (id) {
            root.core("job", [id], function (raw) {
                var r = root.j(raw)
                if (r && r.ok && r.state === "pending" && root.jobs[id] && root.jobs[id].progress !== r.progress) {
                    var jp = root.jobs; jp[id].progress = r.progress || ""; root.jobs = jp
                }
                if (r && (r.state === "done" || r.state === "failed" || !r.ok)) {
                    var cb = root.jobCallbacks[id]
                    var js = root.jobs; delete js[id]; root.jobs = js
                    var cbs = root.jobCallbacks; delete cbs[id]; root.jobCallbacks = cbs
                    if (r.ok && r.state === "done") { if (cb && cb.done) cb.done(r.result) }
                    else {
                        var why = (r && r.error) || "The job was lost (was the core restarted?)."
                        root.fail(why)
                        if (cb && cb.fail) cb.fail(why)
                    }
                    root.refresh()
                }
                if (--left === 0) { root.pollBusy = false; root.jobTick++ }
            })
        })
    }
    function progressOf(label) {
        for (var k in root.jobs) if (root.jobs[k].label === label) return root.jobs[k].progress || ""
        return ""
    }
    // Ticks while jobs run, so "Opening… 12s" counts up.
    property double now: Date.now()
    Timer { interval: 1000; running: root.busy; repeat: true; onTriggered: root.now = Date.now() }
    function elapsed(label) {
        for (var k in root.jobs) if (root.jobs[k].label === label) return Math.max(0, Math.round((root.now - root.jobs[k].started) / 1000))
        return 0
    }

    // ── actions ──
    function open(url) {
        var u = String(url || "").trim()
        if (u === "") return
        root.startJob("open", [u], "open", function (doc) {
            root.docv = doc; root.docUrl = u; root.mode = "doc"; root.showSettings = false
            root.stopPresenting(); root.slideIndex = 0; root.tabIndex = 0; root.sheetIndex = 0
            root.refreshNode(); root.refreshStorage()
        })
    }
    readonly property var recentEntry: {
        var rs = root.snap.recent || []
        for (var i = 0; i < rs.length; i++) if (rs[i].url === root.docUrl) return rs[i]
        return null
    }
    // Pinned on the node in use now (a copy on some other node does not count).
    // pinned on this device's node; while the node is stopped ("When needed") the record counts
    readonly property bool kept: root.recentEntry !== null && !!root.recentEntry.kept && (!root.node.peerId || root.recentEntry.keptOn === root.node.peerId)
    readonly property bool inStorage: root.recentEntry !== null && !!root.recentEntry.inStorage
    function keepCopy() {
        root.startJob("keep", [root.docUrl], "keep", function (r) { root.say("Pinned on your IPFS node (" + r.node + "): " + r.blocks + " blocks"); root.refreshNode() })
    }
    function copyToStorage() {
        root.startJob("copy_to_storage", [root.docUrl], "storage", function (r) { root.say("In Logos Storage: " + r.copied + " copied, " + r.alreadyThere + " already there") })
    }
    function refreshNode() { root.core("node_status", [], function (raw) { var r = root.j(raw); if (r && r.ok) root.node = r }) }
    function refreshStorage() { root.core("storage_status", [], function (raw) { var r = root.j(raw); if (r && r.ok) root.storage = r }) }
    // My files: the portal's files (when a backup key was imported) and the local documents
    function showFiles() {
        var ps = root.snap.portals || []
        var known = ps.some(function (x) { return x.portal === root.portal })
        root.showPortal(known ? root.portal : (ps.length ? ps[0].portal : ""))
    }
    function showPortal(p) {
        var changed = root.portal !== p
        root.portal = p; root.portalError = ""; root.mode = "portal"; root.showSettings = false
        if (changed) { root.portalFiles = []; root.portalBackup = ({}); root.portalListedAt = null }
        if (p === "") return
        // The last listing is kept on this device: show it at once, refresh only when old.
        root.core("portal_files", [p], function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok || r.portal !== root.portal) return
            root.portalFiles = r.files || []
            root.portalBackup = r.backup || ({})
            root.portalListedAt = r.listedAt
            if (!r.listedAt || Date.now() - r.listedAt > 15 * 60 * 1000) root.refreshPortal()
        })
    }
    function refreshPortal() {
        var p = root.portal
        root.startJob("list_portal", [p], "list", function (r) {
            if (root.portal === p) { root.portalFiles = r.files; root.portalListedAt = Date.now() }
        }, function (e) { if (root.portal === p) root.portalError = e })
    }
    function backupPortal() {
        var p = root.portal
        root.startJob("backup_portal", [p], "backup", function (r) { root.refreshNode();
            var where = ["this device"]
            if (r.ipfs) where.push("your IPFS node")
            if (r.storage) where.push("Logos Storage")
            root.say("Backed up " + r.fetched + " of " + r.files + " files (" + r.blocks + " blocks) to " + where.join(", ")
                     + (r.failed && r.failed.length ? " - " + r.failed.length + " failed" : "") + (r.notes && r.notes.length ? " - " + r.notes.join("; ") : ""))
            root.core("portal_files", [p], function (raw) { var x = root.j(raw); if (x && x.ok && root.portal === p) root.portalBackup = x.backup || ({}) })
        })
    }
    function newDraft(title, markdown, source) {
        root.act("draft_create", [title, markdown, source || ""], "Draft created", function (r) { root.editDraft(r.draft) })
    }
    function editDraft(d) {
        root.draft = d; root.draftDirty = false; root.mode = "edit"; root.tabIndex = 0; root.showSettings = false
        root.collab = ({ shared: false }); root.collabVersion = ""; root.collabSentText = d.markdown || ""
        titleField.text = d.title; root.loadEditor(d.markdown)
        root.collabActivate()
    }
    // The draft a document came from, if it is one of yours (a recent entry knows its
    // draft; older entries are matched by the links the draft was published under).
    function draftIdFor(url) {
        var rs = root.snap.recent || []
        for (var i = 0; i < rs.length; i++) if (rs[i].url === url && rs[i].draftId) return rs[i].draftId
        var ds = root.snap.drafts || []
        for (var j = 0; j < ds.length; j++) {
            var d = ds[j]
            if (d.link === url || (d.links || []).indexOf(url) >= 0) return d.id
        }
        return ""
    }
    readonly property string docDraftId: { root.snap; return root.draftIdFor(root.docUrl) }
    // Your most recent copy of someone else's document, so "Edit" reopens it
    // instead of making another copy each time.
    function copyIdFor(url) {
        var ds = root.snap.drafts || [], best = null
        for (var j = 0; j < ds.length; j++)
            if (url && ds[j].sourceUrl === url && (!best || (ds[j].updatedAt || 0) > (best.updatedAt || 0))) best = ds[j]
        return best ? best.id : ""
    }
    readonly property string docCopyId: { root.snap; return root.copyIdFor(root.docUrl) }
    function openDraft(id) { root.act("draft_get", [id], "", function (r) { root.editDraft(r.draft) }) }
    // The draft as last loaded or saved here: a save from an older version is refused by
    // the core (an agent, or another window, wrote it meanwhile).
    function draftBase() { return root.draft && root.draft.updatedAt ? String(root.draft.updatedAt) : "" }
    property var outsideDraft: null       // the newer version on disk while you have unsaved edits
    property bool savingDraft: false
    function saveDraft(then) {
        if (!root.draft || root.outsideDraft) return
        if (root.isShared) { root.saveShared(then); return }
        var visual = root.visualEdit
        root.savingDraft = true
        root.core(visual ? "draft_save_rich" : "draft_save", [root.draft.id, titleField.text, visual ? richArea.text : mdArea.text, root.draftBase()], function (raw) {
            root.savingDraft = false
            var r = root.j(raw)
            if (r && r.ok) { root.draft = r.draft; root.draftDirty = false; root.refresh(); if (then) then() }
            else if (r && r.conflict) root.changedOutside(r.draft)
            else root.fail(r ? r.error : "Saving failed.")
        })
    }
    // ── shared drafts: a draft is private until shared; a shared draft is a Reflection
    // note, edited through reflection_core (which publishes it on the networks set in
    // Settings). The open one is polled for others' edits, presence and cursors.
    property var collab: ({ shared: false })   // the open note's state (see fromReflection)
    property string collabVersion: ""           // the version the editor's text is based on
    property string collabSentText: ""          // the Markdown the editor showed at that version
    property int editGen: 0                     // bumps on every edit: a reply for older text is not applied
    readonly property bool isShared: !!(root.draft && root.collab && root.collab.shared)
    readonly property var collabOnline: (root.collab.peers || []).filter(function (p) { return p.online })
    // Settings hides the other pages, so going to one closes it
    onModeChanged: { root.showSettings = false; root.collabActivate() }
    // ── Reflection notes: a draft linked to one is edited through reflection_core (Loro,
    // Logos Delivery or p2panda), the same note Reflection's own apps edit ──
    readonly property string noteId: root.draft && root.draft.reflection && root.draft.reflection.id ? String(root.draft.reflection.id) : ""
    readonly property bool viaReflection: root.noteId !== ""
    property bool reflectionAvailable: false
    property var reflNotes: []
    property bool reflActive: false
    property string reflSyncMode: ""          // we told reflection_core a note of ours is open
    // through our core (which Basecamp loads with reflection_core when it is installed)
    function refl(method, args, cb) { root.core("reflection_call", [method, JSON.stringify(args || [])], cb) }
    // Is reflection_core installed, and which notes does it have? (Asks for no open note:
    // only when none of ours is open.)
    function probeReflection(cb) {
        root.refl("state", [""], function (raw) {
            var r = root.j(raw)
            root.reflectionAvailable = !!(r && r.ok)
            root.reflNotes = r && r.docs ? r.docs : []
            if (r && r.settings) { root.reflSettings = r.settings; root.reflSyncMode = r.syncMode || "" }
            root.reflActive = false
            if (cb) cb(r)
        })
    }
    function colorOf(key) {
        var palette = ["#E8743B", "#19A979", "#945ECF", "#13A4B4", "#ED4A7B", "#6C8893", "#D6BA00", "#5899DA"], h = 0
        key = String(key || "")
        for (var i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) % 1000003
        return palette[h % palette.length]
    }
    // Fileverse writes an empty line as a `&nbsp;` paragraph; Reflection shows Markdown as
    // typed, so on the note that line is a single space (looks empty, and Markdown keeps
    // it out of the paragraphs around it). Not a no-break space: Reflection's editor reads
    // one back as a space, its text then never matches the note's again, and it stops
    // showing other people's edits. Any line of only blanks comes back as an empty line.
    // `lineSwap` turns lines matching `from` (a string or a RegExp) into `to` and maps
    // offsets across.
    function lineSwap(s, from, to) {
        var lines = String(s || "").split("\n"), out = [], segs = [], src = 0, dst = 0, fence = false
        for (var i = 0; i < lines.length; i++) {
            var l = lines[i]
            if (/^ {0,3}(```|~~~)/.test(l)) fence = !fence   // code blocks keep their lines as they are
            else if (!fence && (typeof from === "string" ? l === from : from.test(l))) { segs.push([src, l.length, dst, to.length]); l = to }
            out.push(l); src += lines[i].length + 1; dst += l.length + 1
        }
        return { text: out.join("\n"), at: function (off) {
            var delta = 0
            for (var k = 0; k < segs.length; k++) {
                var g = segs[k]
                if (off >= g[0] + g[1]) { delta = g[2] + g[3] - g[0] - g[1]; continue }
                if (off > g[0]) { var rel = off - g[0]; return g[2] + (rel >= g[1] ? g[3] : Math.min(rel, g[3])) }
                break
            }
            return off + delta
        } }
    }
    function toNote(md) { return root.lineSwap(md, "&nbsp;", " ") }
    function fromNote(text) { return root.lineSwap(text, /^[ \t\u00a0]+$/, "&nbsp;") }
    // reflection_core's state of the open note, in collab_state's shape
    // Which networks reflection_core publishes on: its `networks` setting ("logos",
    // "p2panda", "both"), or, before it had one, its sync mode.
    property var reflSettings: null             // reflection_core's settings, from its last state
    function networksOf(r) {
        // what the running node uses (reflection_core 0.6: `networks`, briefly behind the
        // setting while it switches), else the setting, else 0.5's sync mode
        if (r && r.networks) return String(r.networks)
        var st = r && r.settings
        if (st && st.networks) return String(st.networks)
        return r && r.syncMode === "p2panda" ? "p2panda" : "logos"
    }
    readonly property bool networksSettable: !!(root.reflSettings && root.reflSettings.networks !== undefined)
    // "Logos: connected, 4 peers · p2panda: 2 peers" from reflection_core 0.6's networkStatus
    function networkStatusText(ns) {
        if (!ns) return ""
        var out = []
        if (ns.logos && ns.logos.used) out.push("Logos: " + (ns.logos.connected ? "connected" : (ns.logos.status || "not connected")) + (ns.logos.peers !== undefined ? ", " + ns.logos.peers + " peers" : ""))
        if (ns.p2panda && ns.p2panda.used) out.push("p2panda: " + (ns.p2panda.error ? ns.p2panda.error : (ns.p2panda.connected ? "connected" : "not connected")) + (ns.p2panda.peers !== undefined ? ", " + ns.p2panda.peers + " peers in the last 5 min" : ""))
        return out.join(" · ")
    }
    function networksLabel(n) { return n === "both" ? "Logos + p2panda" : n === "p2panda" ? "p2panda" : "Logos" }
    function fromReflection(r) {
        var c = r && r.current
        if (!c) return { shared: false, engine: "reflection", missing: true }
        var cursors = c.cursors || [], online = c.online || [], seen = {}, peers = [], md = root.fromNote(c.text || "")
        online.concat(cursors.map(function (x) { return x.author })).forEach(function (a) {
            if (!a || seen[a]) return
            seen[a] = true
            var cur = cursors.filter(function (x) { return x.author === a })[0]
            peers.push({ id: a, name: String(a).substring(0, 6), color: root.colorOf(a), online: true,
                         caret: cur ? md.at(cur.pos) : null, anchor: cur ? md.at(cur.anchor) : null })
        })
        return { ok: true, shared: true, engine: "reflection", id: c.id, name: c.name || "", networks: root.networksOf(r), networkStatus: r.networkStatus || null, delivery: c.delivery || null, version: String(c.version), text: md.text,
                 peers: peers, transport: r.transport, joined: true,
                 me: { id: r.me, name: "you", speed: "live", shareCursor: !!(r.settings && r.settings.shareCursor) } }
    }
    function collabActivate() {
        var editing = root.mode === "edit" && root.draft
        var id = editing ? root.draft.id : ""
        // a Reflection note was open here and no longer is: reflection_core may go quiet
        if (root.reflActive && !(editing && root.viaReflection)) { root.refl("state", [""], function () {}); root.reflActive = false }
        if (!(editing && root.viaReflection)) root.collab = ({ shared: false })
        root.refreshCollab()
    }
    function collabStateOf(id, cb) {
        if (root.viaReflection) {
            var note = root.noteId
            root.reflActive = true
            root.refl("state", [note], function (raw) {
                var r = root.j(raw)
                if (!r || !r.ok) { cb({ ok: true, shared: false, engine: "reflection", missing: true }); return }
                // the open note's poll also lists the notes (the join panel needs no probe then)
                root.reflectionAvailable = true
                if (r.docs) root.reflNotes = r.docs
                if (r.settings) { root.reflSettings = r.settings; root.reflSyncMode = r.syncMode || "" }
                cb(root.fromReflection(r))
            })
        } else cb({ ok: true, shared: false })
    }
    // the draft file follows the shared text (slides, preview, publishing, agents read it)
    function mirrorDraft(md) {
        if (!root.draft || root.draft.markdown === md) return
        var id = root.draft.id
        root.core("draft_save", [id, titleField.text, md, ""], function () {})
    }
    function refreshCollab() {
        if (root.mode !== "edit" || !root.draft) return
        var id = root.draft.id
        root.collabStateOf(id, function (r) {
            if (!r || !r.ok || !root.draft || root.draft.id !== id) return
            var first = !root.collab.shared && r.shared
            root.collab = r
            if (!r.shared) return
            // a draft made for a note takes the note's name (Reflection names it by its heading)
            if (r.name && r.name !== titleField.text && /^(Untitled|Reflection note)?$/.test(titleField.text)) {
                titleField.text = r.name
                root.core("draft_save", [id, r.name, root.draft.markdown, ""], function () {})
            }
            if (first && root.collabVersion === "") {
                // the editor shows the draft file; the shared text is the truth
                root.collabVersion = r.version
                if (r.text !== root.collabSentText) root.showSharedText(r.text)
                else root.collabSentText = r.text
                return
            }
            root.applyRemote(r)
        })
    }
    readonly property bool collabLive: true
    Timer { interval: root.collabLive ? 300 : 1000; running: root.mode === "edit" && root.isShared; repeat: true; onTriggered: root.refreshCollab() }
    // Others' edits: show them unless the user is mid-edit (then the next save merges).
    function applyRemote(r) {
        if (root.draftDirty || root.savingDraft || previewTimer.running) return
        if (r.version === root.collabVersion) return
        // a reply to a poll sent before our last save can arrive after it: only a newer
        // version than the editor's may replace the editor's text (else typed keys vanish)
        if (root.collabVersion !== "" && Number(r.version) <= Number(root.collabVersion)) return
        root.collabVersion = r.version
        if (r.text !== root.collabSentText) root.showSharedText(r.text)
    }
    // Put the shared text in the editor, keeping the caret by the text before it.
    function showSharedText(md) {
        var visual = root.visualEdit
        var full = visual ? richArea.getText(0, richArea.length) : mdArea.text
        var pos = visual ? richArea.cursorPosition : mdArea.cursorPosition
        var before = full.substring(0, pos)
        var tail = before.substring(root.lastBreak(before) + 1)
        root.collabSentText = md
        root.loadEditor(md)
        if (visual) richArea.cursorPosition = root.cursorFor(richArea.getText(0, richArea.length), tail, full.length ? pos / full.length : 0)
        else mdArea.cursorPosition = root.cursorFor(mdArea.text, tail, full.length ? pos / full.length : 0)
        root.draftDirty = false
        root.updatePreview(md)
        if (root.viaReflection) root.mirrorDraft(md)
    }
    function updatePreview(md) {
        var id = root.draft ? root.draft.id : ""
        root.core("preview", [md], function (raw) {
            var p = root.j(raw)
            if (!p || !p.ok || !root.draft || root.draft.id !== id) return
            var d = JSON.parse(JSON.stringify(root.draft)); d.preview = p.tabs; d.markdown = md
            root.draft = d
        })
    }
    function saveShared(then) {
        // one save at a time; typing during it is sent right after
        if (root.savingDraft) { previewTimer.start(); return }
        var visual = root.visualEdit, id = root.draft.id, gen = root.editGen
        var titleChanged = titleField.text !== root.draft.title
        root.savingDraft = true
        root.sharedSetText(visual, function (r) {
            root.savingDraft = false
            if (!r || !r.ok) { root.fail(r ? r.error : "Saving failed."); return }
            if (!root.draft || root.draft.id !== id) return
            if (gen === root.editGen) {
                // nothing typed meanwhile: show the merged text
                root.collabVersion = r.version
                root.draftDirty = false
                if (r.merged) root.showSharedText(r.text)
                else { root.collabSentText = r.text; root.updatePreview(r.text); if (root.viaReflection) root.mirrorDraft(r.text) }
            } else {
                // typed on: the editor builds on what was sent (others' changes show at the
                // next pause), and the rest goes out shortly
                root.collabVersion = r.editVersion
                root.collabSentText = r.submitted
                if (!previewTimer.running) previewTimer.start()
            }
            // the title is this device's name for the draft (not shared)
            root.sendCursor()
            if (titleChanged) root.core("draft_save", [id, titleField.text, r.text, ""], function (raw2) { var t = root.j(raw2); if (t && t.ok && root.draft && root.draft.id === id) { var d = JSON.parse(JSON.stringify(root.draft)); d.title = t.draft.title; root.draft = d } })
            if (then) then()
        })
    }
    // Send the editor's text to the draft's engine: `{ ok, version, text, merged, editVersion, submitted }`.
    function sharedSetText(visual, cb) {
        var id = root.draft.id
        var md = visual ? ((root.coreSync("rich_md", [id, richArea.text]) || {}).markdown) : mdArea.text
        if (md === undefined || md === null) { cb({ ok: false, error: "Could not convert the editor's text." }); return }
        var note = root.toNote(md).text
        root.refl("set_text", [root.noteId, root.collabVersion, note], function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok || !r.current) { cb({ ok: false, error: (r && r.error) || "reflection_core did not answer" }); return }
            var c = r.current
            cb({ ok: true, version: String(c.version), text: root.fromNote(c.text).text, merged: c.text !== note,
                 editVersion: String(c.editVersion !== undefined ? c.editVersion : c.version), submitted: md })
        })
    }
    // "Share…": with Fileverse (HashSeq), or as a Reflection note when reflection_core is here
    property bool shareOpen: false
    function shareClicked() {
        if (!root.draft) return
        root.probeReflection(function () {
            if (root.reflectionAvailable) root.shareAsNote()
            else root.fail("Sharing goes through Reflection's engine (reflection_core), which is not answering. Is it installed?")
        })
    }
    function shareAsNote() {
        if (!root.draft) return
        root.shareOpen = false
        var draftId = root.draft.id, title = titleField.text || root.draft.title || "Untitled"
        root.saveDraft(function () {
            var md = root.draft.markdown, text = root.toNote(md).text
            // reflection_core takes at most 100 KB of text (a full copy must fit one message):
            // say so here instead of sharing a note that holds only the title
            var bytes = unescape(encodeURIComponent(text)).length
            if (bytes > 100 * 1024) {
                root.fail("This document is too long to share (" + Math.ceil(bytes / 1024) + " KB of text, at most 100 KB). Split it, or share a shorter part.")
                return
            }
            root.refl("create_document", [title], function (raw) {
                var r = root.j(raw)
                if (!r || !r.ok) { root.fail((r && r.error) || "reflection_core could not create a note"); return }
                var note = r.id
                root.refl("set_text", [note, String(r.current.version), text], function (raw2) {
                    // only a note that took the whole text is linked and announced
                    var w = root.j(raw2)
                    if (!w || !w.ok) { root.fail("Could not share the text: " + ((w && w.error) || "reflection_core did not answer") + ". Nothing was shared."); return }
                    root.core("reflection_link", [draftId, note], function () {
                        if (!root.draft || root.draft.id !== draftId) return
                        var d = JSON.parse(JSON.stringify(root.draft)); d.reflection = { id: note }; root.draft = d
                        root.collab = ({ shared: false }); root.collabVersion = ""; root.collabSentText = md
                        root.copy(note, "Reflection note id")
                        root.say("Shared. Its id is copied: send it to whoever should edit with you (Fileverse, or the Reflection apps). Anyone with it can read and write.")
                        root.collabActivate()
                    })
                })
            })
        })
    }
    // Open a Reflection note as a draft: the draft linked to it, or a new one.
    function openNote(n) {
        root.joinOpen = false
        var ds = root.snap.drafts || []
        for (var i = 0; i < ds.length; i++) if (ds[i].reflection === n.id) { root.section = "drafts"; root.openDraft(ds[i].id); return }
        root.refl("state", [n.id], function (raw) {
            var r = root.j(raw), c = r && r.current
            var text = root.fromNote(c ? (c.text || "") : "").text
            root.act("draft_create", [n.name || "Reflection note", text, ""], "", function (cr) {
                root.core("reflection_link", [cr.draft.id, n.id], function () {
                    var d = cr.draft; d.reflection = { id: n.id }
                    root.section = "drafts"
                    root.editDraft(d)
                })
            })
        })
    }
    function joinNote(id) {
        root.refl("join_document", [String(id).trim()], function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok) { root.fail((r && r.error) || "reflection_core could not join that note"); return }
            root.openNote({ id: r.id, name: r.current && r.current.name ? r.current.name : "Reflection note" })
        })
    }
    function stopSharing() {
        if (!root.draft) return
        var id = root.draft.id
        if (root.viaReflection) {
            root.act("reflection_link", [id, ""], "Stopped sharing: the document is private here; the note stays in Reflection", function () {
                var d = JSON.parse(JSON.stringify(root.draft)); delete d.reflection; root.draft = d
                root.collabActivate(); root.collab = ({ shared: false }); root.collabVersion = ""
            })
        }
    }
    property bool joinOpen: false
    function joinShared(sharedId) {
        root.probeReflection(function () {
            if (root.reflectionAvailable) root.joinNote(sharedId)
            else root.fail("Joining goes through Reflection's engine (reflection_core), which is not answering. Is it installed?")
        })
    }
    // Our cursor, as offsets into the shared Markdown (in the visual editor, found by the
    // text before the caret).
    Timer { id: cursorTimer; interval: root.collabLive ? 120 : 300; onTriggered: root.sendCursor() }
    function cursorMoved() { if (root.isShared) cursorTimer.restart() }
    // Markdown -> the text the visual editor shows: block markers, emphasis marks, escapes
    // and link targets removed, paragraphs as one line break. `map[i]` is the Markdown
    // offset of visible character i (and map[text.length] the end).
    function visibleMap(md) {
        var out = [], map = [], lines = String(md).split("\n"), off = 0, fence = false
        for (var li = 0; li < lines.length; li++) {
            var t = lines[li], start = off
            off += t.length + 1
            if (/^\s*```/.test(t)) { fence = !fence; continue }
            var i = 0
            if (!fence) {
                // an empty line made in the editor (`&nbsp;`): an empty paragraph there
                if (t.trim() === "&nbsp;") { if (out.length) { out.push("\n"); map.push(start) } continue }
                if (/^\s*$/.test(t) || /^\s*:::/.test(t) || /^\s*(---|===+)\s*$/.test(t)) continue
                var m = t.match(/^\s*(#{1,6}\s+|>\s?|[-*+]\s+(\[[ xX]\]\s+)?|\d+[.)]\s+)?/)
                i = m ? m[0].length : 0
            }
            if (out.length) { out.push("\n"); map.push(start) }
            while (i < t.length) {
                var c = t.charAt(i)
                if (!fence) {
                    if (c === "\\" && i + 1 < t.length) { out.push(t.charAt(i + 1)); map.push(start + i + 1); i += 2; continue }
                    if (c === "*" || c === "`" || c === "~") { i++; continue }
                    if (c === "!" && t.charAt(i + 1) === "[") { var e = t.indexOf(")", i); if (e > 0) { out.push("\ufffc"); map.push(start + i); i = e + 1; continue } }
                    if (c === "]" && t.charAt(i + 1) === "(") { var e2 = t.indexOf(")", i); if (e2 > 0) { i = e2 + 1; continue } }
                    if (c === "[" && t.indexOf("](", i) > 0) { i++; continue }
                }
                out.push(c); map.push(start + i); i++
            }
        }
        map.push(String(md).length)
        return { text: out.join(""), map: map }
    }
    // The visual editor's text with every block break as one line break (same length).
    function richPlain() { return richArea.getText(0, richArea.length).replace(/[\u2029\ufdd0\ufdd1]/g, "\n") }
    // A Markdown offset -> where that place is in the visual editor (by the visible text
    // just before it).
    function mdToRich(md, mdOffset) {
        var vm = root.visibleMap(md), v = 0
        while (v < vm.text.length && vm.map[v] < mdOffset) v++
        var plain = root.richPlain()
        if (v === 0) return 0
        return root.cursorFor(plain, vm.text.substring(Math.max(0, v - 20), v), vm.text.length ? v / vm.text.length : 0)
    }
    // A place in the visual editor -> its Markdown offset.
    function richToMd(md, pos) {
        var vm = root.visibleMap(md), plain = root.richPlain()
        if (pos <= 0) return vm.map[0] || 0
        var v = root.cursorFor(vm.text, plain.substring(Math.max(0, pos - 20), pos), plain.length ? pos / plain.length : 0)
        return vm.map[Math.min(v, vm.map.length - 1)]
    }
    function sendCursor() {
        if (!root.isShared || !root.draft) return
        var md = root.collabSentText, caret, anchor
        if (!root.visualEdit) {
            caret = mdArea.cursorPosition
            anchor = mdArea.selectionStart === mdArea.cursorPosition ? mdArea.selectionEnd : mdArea.selectionStart
        } else {
            caret = root.richToMd(md, richArea.cursorPosition)
            var other = richArea.selectionStart === richArea.cursorPosition ? richArea.selectionEnd : richArea.selectionStart
            anchor = other === richArea.cursorPosition ? caret : root.richToMd(md, other)
        }
        if (!root.viaReflection) return
        var nm = root.toNote(md)
        root.refl("set_cursor", [root.noteId, root.collabVersion, String(nm.at(caret)), String(nm.at(anchor))], function () {})
    }
    // Where to draw someone's caret: offsets come against the shared text as the core has
    // it now (collab.text).
    function peerCaretPos(caret) {
        if (caret === null || caret === undefined) return -1
        var md = root.collab.text !== undefined ? root.collab.text : root.collabSentText
        if (!root.visualEdit) return Math.min(caret, mdArea.length)
        return root.mdToRich(md, caret)
    }
    function changedOutside(d) {
        if (!root.draft || !d || d.id !== root.draft.id) return
        // our own save, seen before its reply: nothing changed outside
        if (!root.visualEdit && d.markdown === mdArea.text && d.title === titleField.text) { root.draft = d; return }
        if (!root.draftDirty) root.reloadDraft(d)
        else root.outsideDraft = d
    }
    // Show a newer version of the open draft, keeping the cursor about where it was.
    function reloadDraft(d) {
        var visual = root.visualEdit, pos = visual ? richArea.cursorPosition : mdArea.cursorPosition
        root.outsideDraft = null
        root.draft = d; root.draftDirty = false
        titleField.text = d.title
        root.loadEditor(d.markdown)
        if (visual) richArea.cursorPosition = Math.min(pos, richArea.length)
        else mdArea.cursorPosition = Math.min(pos, mdArea.length)
        root.say("Updated: the document was changed outside the editor")
    }
    function keepMine() {
        // overwrite the outside version with what is in the editor
        if (!root.outsideDraft) return
        var d = root.draft
        d.updatedAt = root.outsideDraft.updatedAt
        root.draft = d
        root.outsideDraft = null
        root.saveDraft()
    }
    // Each snapshot lists the drafts on disk: notice when the open one was written elsewhere.
    function checkOutside() {
        if (root.mode !== "edit" || !root.draft || root.outsideDraft || root.savingDraft || previewTimer.running || root.isShared) return
        var ds = root.snap.drafts || []
        for (var i = 0; i < ds.length; i++) {
            if (ds[i].id !== root.draft.id) continue
            if ((ds[i].updatedAt || 0) > (root.draft.updatedAt || 0))
                root.core("draft_get", [root.draft.id], function (raw) { var r = root.j(raw); if (r && r.ok && r.draft.updatedAt > (root.draft ? root.draft.updatedAt || 0 : 0)) root.changedOutside(r.draft) })
            return
        }
    }
    // ── the visual editor ──
    // The TextArea edits formatted text in Qt's RichText mode; drafts stay Markdown.
    // The core converts both ways (rich_html / draft_save_rich) and restyles blocks
    // (rich_block), because QML has no block-format API and Qt's own Markdown writer
    // garbles code blocks and tables. Inline styles go through `cursorSelection.font`.
    property bool visualEdit: true
    property bool loadingEditor: false
    property string blockKind: "p"        // p h1 h2 h3 ul ol task done quote code hr table
    function loadEditor(md) {
        root.commitStep()
        root.loadingEditor = true
        mdArea.text = md
        root.loadingEditor = false
        if (!root.visualEdit) { root.histLoaded(); return }
        var r = root.coreSync("rich_html", [md, root.draft ? root.draft.id : "", root.viewDir])
        if (!r || !r.ok) { root.fail("Could not open the visual editor; showing Markdown."); root.visualEdit = false; return }
        root.loadingEditor = true
        richArea.text = root.tintLinks(r.html)
        root.loadingEditor = false
        richArea.cursorPosition = 0
        root.updateBlockKind()
        root.histLoaded()
        root.fetchEditorImages(r.pending || [])
        editorRedraw.restart()
    }
    // After the visual editor's text is replaced, Qt sometimes leaves part of it undrawn
    // (vpavlin, after AI edits: the top of the document blank while the Markdown had it;
    // an undo, which sets the text again, brought it back). When the layout and scrolling
    // have settled, set the same text once more, keeping the cursor, the selection, the scroll
    // position and the undo history.
    Timer {
        id: editorRedraw
        interval: 150
        onTriggered: {
            if (!root.visualEdit || root.loadingEditor) return
            var f = richScroll.contentItem, y = f ? f.contentY : 0
            var a = richArea.selectionStart, b = richArea.selectionEnd, c = richArea.cursorPosition
            var html = richArea.text, base = root.histBase === html
            root.loadingEditor = true; root.histApplying = true
            richArea.text = html
            root.histApplying = false; root.loadingEditor = false
            if (a !== b) richArea.select(c === a ? b : a, c)
            else richArea.cursorPosition = Math.min(c, richArea.length)
            if (f) f.contentY = y
            if (base) root.histBase = richArea.text
        }
    }
    // Images the editor shows but the cache does not have yet (a copy of a Fileverse
    // document): fetch them, then show the document again where the cursor was.
    function fetchEditorImages(pending) {
        if (!pending.length) return
        var left = pending.length
        var draftId = root.draft ? root.draft.id : ""
        pending.forEach(function (ref) {
            root.quietJob("image", [JSON.stringify(ref), root.viewDir], finish, finish)
        })
        function finish() {
            if (--left > 0 || !root.draft || root.draft.id !== draftId || !root.visualEdit) return
            var pos = richArea.cursorPosition
            root.loadingEditor = true
            // Qt remembers that these files were missing: a new query makes it load them now
            var stamp = "?r=" + Date.now()
            richArea.text = richArea.text.replace(/(src="[^"?]*\/imgcache\/[^"?]*)(\?r=\d+)?"/g, "$1" + stamp + '"')
            root.loadingEditor = false
            richArea.cursorPosition = Math.min(pos, richArea.length)
        }
    }
    function insertImage(path) {
        if (!root.draft) return
        var r = root.coreSync("draft_add_image", [root.draft.id, String(path), root.viewDir])
        if (!r || !r.ok) { root.fail(r ? r.error : "Could not add the picture."); return }
        if (root.visualEdit) {
            richArea.insert(richArea.cursorPosition, r.html)
            richArea.forceActiveFocus()
        } else {
            mdArea.insert(mdArea.cursorPosition, "\n\n" + r.md + "\n\n")
        }
        root.editorChanged()
    }
    // Two columns side by side (dDocs' image beside text): a borderless table in the editor.
    function insertColumns() {
        if (root.visualEdit) {
            if (root.inTable()) { root.say("Columns cannot go inside a table or other columns"); return }
            richArea.insert(richArea.cursorPosition, '<table border="0" width="100%" cellpadding="8"><tr><td width="50%" valign="top"><p>Text here</p></td><td width="50%" valign="top"><p>Image or text here</p></td></tr></table><p></p>')
            richArea.forceActiveFocus()
        } else {
            mdArea.insert(mdArea.cursorPosition, "\n\n::: columns align-center\n\nText here\n\n::: column\n\nImage or text here\n\n:::\n\n")
        }
        root.editorChanged()
    }
    function setVisualEdit(v) {
        if (v === root.visualEdit || !root.draft) return
        if (v) {
            var mp = mdArea.cursorPosition, mt = mdArea.text
            var line = mt.substring(mt.lastIndexOf("\n", mp - 1) + 1, mp)
            // what you see of that line: no block markers or inline marks
            var seen = line.replace(/^\s*(#{1,6}\s|[-*+]\s(\[[ xX]\]\s)?|\d+[.)]\s|>\s?)/, "").replace(/[*_`~\\]/g, "")
            root.visualEdit = true
            root.loadEditor(mt)
            richArea.cursorPosition = root.cursorFor(richArea.getText(0, richArea.length), seen, mt.length ? mp / mt.length : 0)
            richArea.forceActiveFocus()
            return
        }
        var rp = richArea.cursorPosition, rt = richArea.getText(0, richArea.length)
        var before = rt.substring(0, rp)
        var seenR = before.substring(root.lastBreak(before) + 1)
        // visual -> markdown: save converts, and hands the Markdown back
        root.core("draft_save_rich", [root.draft.id, titleField.text, richArea.text, root.draftBase()], function (raw) {
            var r = root.j(raw)
            if (r && r.conflict) { root.changedOutside(r.draft); return }
            if (!r || !r.ok) { root.fail(r ? r.error : "Saving failed."); return }
            root.draft = r.draft; root.draftDirty = false
            root.visualEdit = false
            root.loadEditor(r.draft.markdown)
            mdArea.cursorPosition = root.cursorFor(mdArea.text, seenR, rt.length ? rp / rt.length : 0)
            mdArea.forceActiveFocus()
        })
    }
    // Where the cursor goes after switching views: just after the text it followed (the
    // last few characters of its line, the occurrence nearest the same relative place),
    // else the same relative place.
    function cursorFor(text, seen, frac) {
        var approx = Math.round(frac * text.length)
        var needle = String(seen).replace(/\u200b/g, "")
        needle = needle.substring(Math.max(0, needle.length - 24))
        if (needle.trim().length >= 2) {
            var best = -1, at = text.indexOf(needle)
            while (at >= 0) {
                if (best < 0 || Math.abs(at - approx) < Math.abs(best - approx)) best = at
                at = text.indexOf(needle, at + 1)
            }
            if (best >= 0) return best + needle.length
        }
        return Math.max(0, Math.min(approx, text.length))
    }
    // ── undo: Fileverse's own history, in steps of a word or a pause, as Google Docs,
    // LibreOffice and ddocs.new undo. A step keeps only what it changed and a little text
    // around it, so undo finds that place in the current text and puts back what was there:
    // in a shared document the others' edits stay. Text arriving from others or loaded does
    // not clear it (Qt's own undo forgets everything then).
    property var undoStack: []
    property var redoStack: []
    property string histBase: ""        // the editor's text when the last step closed
    property bool stepOpen: false
    property int stepCaret: 0
    property bool histApplying: false
    property string histKey: ""         // draft id + editor kind the history belongs to
    function histText() { return root.visualEdit ? richArea.text : mdArea.text }
    // after the editor's text was loaded (opening, a remote edit, switching editors)
    function histLoaded() {
        var key = (root.draft ? root.draft.id : "") + (root.visualEdit ? ":v" : ":m")
        if (key !== root.histKey) { root.undoStack = []; root.redoStack = []; root.histKey = key }
        root.stepOpen = false
        root.histBase = root.histText()
    }
    function histEdited() {
        if (root.histApplying) return
        if (!root.stepOpen) {
            root.stepOpen = true
            root.stepCaret = root.visualEdit ? richArea.cursorPosition : mdArea.cursorPosition
        }
        stepTimer.restart()
    }
    Timer { id: stepTimer; interval: 700; onTriggered: root.commitStep() }
    function commitStep() {
        stepTimer.stop()
        if (!root.stepOpen) return
        root.stepOpen = false
        var a = root.histBase, b = root.histText()
        root.histBase = b
        if (a === b) return
        var p = 0, n = Math.min(a.length, b.length)
        while (p < n && a.charCodeAt(p) === b.charCodeAt(p)) p++
        var q = 0
        while (q < n - p && a.charCodeAt(a.length - 1 - q) === b.charCodeAt(b.length - 1 - q)) q++
        var st = root.undoStack.slice()
        st.push({ removed: a.substring(p, a.length - q), inserted: b.substring(p, b.length - q),
                  anchor: b.substring(Math.max(0, p - 48), p), after: b.substring(b.length - q, Math.min(b.length, b.length - q + 48)),
                  caret: root.stepCaret })
        if (st.length > 200) st.shift()
        root.undoStack = st; root.redoStack = []
    }
    // Put `step.removed` back where `step.inserted` is now; returns the inverse step.
    function applyStep(step) {
        var t = root.histText(), at = -1
        var tries = [[step.anchor + step.inserted + step.after, step.anchor.length], [step.anchor + step.inserted, step.anchor.length], [step.inserted + step.after, 0]]
        for (var k = 0; k < tries.length && at < 0; k++) {
            if (!tries[k][0].length) continue
            var i = t.indexOf(tries[k][0])
            if (i >= 0) at = i + tries[k][1]
        }
        if (at < 0) return null
        var next = t.substring(0, at) + step.removed + t.substring(at + step.inserted.length)
        var area = root.visualEdit ? richArea : mdArea
        var plainOf = function () { return root.visualEdit ? richArea.getText(0, richArea.length) : mdArea.text }
        var caret = area.cursorPosition, before = plainOf()
        root.histApplying = true
        if (root.visualEdit) richArea.text = next; else mdArea.text = next
        root.histApplying = false
        // the cursor goes where the text changed (the end of what came back, or where the
        // removed text was), not to the start or end of the document
        var after = plainOf(), p = 0, n = Math.min(before.length, after.length)
        while (p < n && before.charCodeAt(p) === after.charCodeAt(p)) p++
        var q = 0
        while (q < n - p && before.charCodeAt(before.length - 1 - q) === after.charCodeAt(after.length - 1 - q)) q++
        // (a formatting change leaves the text the same: the cursor stays where it was)
        area.cursorPosition = Math.max(0, Math.min(before === after ? caret : after.length - q, area.length))
        area.forceActiveFocus()
        root.histBase = root.histText()
        root.editorChanged()
        root.histApplying = false
        return { removed: step.inserted, inserted: step.removed, anchor: step.anchor, after: step.after, caret: caret }
    }
    function undoStep() {
        root.commitStep()
        var st = root.undoStack.slice()
        if (!st.length) return
        var inv = root.applyStep(st.pop())
        root.undoStack = st
        if (!inv) { root.say("That text was changed since; nothing to undo there"); return }
        var r = root.redoStack.slice(); r.push(inv); root.redoStack = r
    }
    function redoStep() {
        root.commitStep()
        var r = root.redoStack.slice()
        if (!r.length) return
        var inv = root.applyStep(r.pop())
        root.redoStack = r
        if (!inv) { root.say("That text was changed since; nothing to redo there"); return }
        var st = root.undoStack.slice(); st.push(inv); root.undoStack = st
    }
    function editorChanged() {
        if (root.loadingEditor) return
        root.histEdited()
        root.editGen++
        root.draftDirty = true
        // a shared draft sends while you type (every timer period), not only when you pause
        if (root.isShared) { if (!previewTimer.running) previewTimer.start() }
        else previewTimer.restart()
    }
    // Which text block the cursor is in, counted as Qt counts them (paragraph
    // separators; table cells are separated by other characters).
    // Qt's plain text splits blocks with U+2029 and marks tables with U+FDD0 (before each
    // cell) and U+FDD1 (after the last); the core counts the same pieces.
    readonly property var pieceBreak: /[\u2029\ufdd0\ufdd1]/
    // The editor's text up to `pos`. Not getText(0, pos): a range that ends inside a
    // table is widened by Qt to the whole table. Positions in the full text match.
    function textBefore(pos) {
        return richArea.getText(0, richArea.length).substring(0, pos === undefined ? richArea.cursorPosition : pos)
    }
    // The cursor's text piece for the core, with one flag per table: is the piece right
    // after it empty ("1"; Qt keeps an empty block there) or typed into ("0"). E.g. "7:10".
    function blockIndex() {
        var full = richArea.getText(0, richArea.length)
        var n = full.substring(0, richArea.cursorPosition).split(root.pieceBreak).length - 1
        var flags = ""
        for (var i = full.indexOf("\ufdd1"); i >= 0; i = full.indexOf("\ufdd1", i + 1))
            flags += (i + 1 >= full.length || root.pieceBreak.test(full.charAt(i + 1))) ? "1" : "0"
        return flags ? n + ":" + flags : String(n)
    }
    // The text of the cursor's block (without the break characters around it).
    function blockText() {
        var full = richArea.getText(0, richArea.length), c = richArea.cursorPosition
        var s = root.lastBreak(full.substring(0, c)) + 1
        var m = full.substring(c).search(root.pieceBreak)
        return full.substring(s, m < 0 ? full.length : c + m)
    }
    function isBlank(t) { return String(t).replace(/[\u200b\ufeff\u00a0\s]/g, "") === "" }
    // The kind of the block after the cursor's ("" at the end).
    function kindAfter() {
        var parts = String(root.blockIndex()).split(":")
        var next = (Number(parts[0]) + 1) + (parts.length > 1 ? ":" + parts[1] : "")
        var r = root.coreSync("rich_kind", [richArea.text, next])
        return r && r.ok ? String(r.kind) : ""
    }
    function lastBreak(p) {
        return Math.max(p.lastIndexOf("\u2029"), p.lastIndexOf("\ufdd0"), p.lastIndexOf("\ufdd1"))
    }
    function blockStart() {
        return root.lastBreak(root.textBefore()) + 1
    }
    // In a table or columns cell: the last table marker before the cursor opens a cell.
    function inTable() {
        var p = root.textBefore()
        return p.lastIndexOf("\ufdd0") > p.lastIndexOf("\ufdd1")
    }
    Timer { id: kindTimer; interval: 150; onTriggered: root.refreshBlockKind() }
    function updateBlockKind() { if (root.visualEdit) kindTimer.restart() }
    function refreshBlockKind() {
        var r = root.coreSync("rich_kind", [richArea.text, String(root.blockIndex())])
        if (r && r.ok) root.blockKind = r.kind || "p"
    }
    function setBlock(kind) {
        root.commitStep()
        var pos = richArea.cursorPosition
        var r = root.coreSync("rich_block", [richArea.text, String(root.blockIndex()), kind, root.draft ? root.draft.id : "", root.viewDir])
        if (!r || !r.ok) {
            var why = r ? String(r.error) : ""
            if (/keep their layout|inside columns/.test(why)) root.say(why.charAt(0).toUpperCase() + why.substring(1))
            else root.fail(why || "Could not change this block.")
            return
        }
        root.loadingEditor = true
        richArea.text = root.tintLinks(r.html)
        root.loadingEditor = false
        pos = Math.max(0, Math.min(pos, richArea.length))
        // an empty block restyled holds a zero-width space (Qt drops empty headings): type after it
        if (richArea.getText(pos, pos + 1) === "\u200b") pos++
        richArea.cursorPosition = pos
        richArea.forceActiveFocus()
        root.editorChanged()
        Qt.callLater(root.commitStep)
        if (kind === "indent" || kind === "outdent") root.refreshBlockKind()
        else root.blockKind = kind
    }
    // Qt draws links in a fixed dark blue; give them the theme's colour.
    // ── the paper: documents, the editors, the preview and slides can be read on white
    // (Light), on a very dark grey (Dark) or in Basecamp's own colours. Kept as a preference.
    readonly property string paper: (root.snap && root.snap.prefs && root.snap.prefs.paper) ? String(root.snap.prefs.paper) : "basecamp"
    readonly property bool paperLight: root.paper === "light"
    readonly property bool paperDark: root.paper === "dark"
    readonly property color paperBg: root.paperLight ? "#ffffff" : root.paperDark ? "#1c1c1e" : Theme.palette.backgroundSecondary
    readonly property color paperInset: root.paperLight ? "#f7f7f7" : root.paperDark ? "#161618" : Theme.palette.backgroundInset
    readonly property color paperText: root.paperLight ? "#1b1b1b" : root.paperDark ? "#ececec" : Theme.palette.text
    readonly property color paperSub: root.paperLight ? "#5f6368" : root.paperDark ? "#a1a1a6" : Theme.palette.textSecondary
    readonly property color paperFaint: root.paperLight ? "#80868b" : root.paperDark ? "#7c7c80" : Theme.palette.textTertiary
    readonly property color paperCode: root.paperLight ? "#f1f3f4" : root.paperDark ? "#2c2c2e" : Theme.palette.surfaceRaised
    readonly property color paperCodeText: root.paperLight ? "#b3361b" : Theme.palette.primary
    readonly property color paperBorder: root.paperLight ? "#dadce0" : root.paperDark ? "#3a3a3c" : Theme.palette.borderHairline
    readonly property color paperLink: root.paperLight ? "#c2410c" : Theme.palette.primary
    // The IPFS node: "off", "needed" (the default) or "always" (core pref ipfsNode).
    readonly property string ipfsMode: (root.snap && root.snap.prefs && root.snap.prefs.ipfsNode) || "needed"
    // a node Keep / Back up / Publish can use: running, or the IPFS Node module that starts when needed
    readonly property bool ipfsUsable: !!root.node.reachable || (root.ipfsMode !== "off" && !/install/.test(root.node.error || ""))
    function setIpfsMode(mode) {
        root.core("set_pref", ["ipfsNode", JSON.stringify(mode)], function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok) { root.fail((r && r.error) || "Could not save the setting"); return }
            var sn = JSON.parse(JSON.stringify(root.snap)); sn.prefs = r.prefs; root.snap = sn
            root.say(mode === "off" ? "The IPFS node is off" : mode === "needed" ? "The IPFS node starts only when needed" : "The IPFS node stays on")
            root.refreshNode()
        })
    }
    function setPaper(kind) {
        root.core("set_pref", ["paper", JSON.stringify(kind)], function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok) { root.fail((r && r.error) || "Could not save the setting"); return }
            var sn = JSON.parse(JSON.stringify(root.snap)); sn.prefs = r.prefs; root.snap = sn
            // the visual editor holds its colours in its text: show the document again
            if (root.mode === "edit" && root.draft && root.visualEdit) root.saveDraft(function () { root.loadEditor(root.draft.markdown) })
        })
    }
    function linkTint(html) {
        var c = String(root.paperLink)
        return String(html).replace(/<a href="([^"]*)">/g, '<a href="$1"><span style="color:' + c + ';">').replace(/<\/a>/g, "</span></a>")
    }
    function tintLinks(html) { return root.codeTint(root.linkTint(html)) }
    // Inline code stands out: monospace on a tinted background in the accent colour (Qt's
    // rich text knows <code> only as a font; the reading view's styled text not at all).
    function codeStyle() {
        return "font-family:'monospace'; background-color:" + String(root.paperCode) + "; color:" + String(root.paperCodeText) + ";"
    }
    function codeSpan(escaped) { return '<code style="' + root.codeStyle() + '">' + escaped + "</code>" }
    function codeTint(html) { return String(html).replace(/<code>/g, '<code style="' + root.codeStyle() + '">') }
    // for read-only text (documents, preview, slides, tables): a little room around it too
    function codeRead(html) {
        return String(html).replace(/<code>/g, '<span style="' + root.codeStyle() + '">&nbsp;').replace(/<\/code>/g, "&nbsp;</span>")
    }
    // Links open through the core: Basecamp's QML sandbox stops Qt.openUrlExternally.
    function openLink(url) {
        url = String(url)
        if (!/^(https?:\/\/|mailto:)/i.test(url)) { root.say("Only web and mail links open"); return }
        root.core("open_link", [url], function (raw) {
            var r = root.j(raw)
            if (r && r.ok) { root.say("Opening " + (url.length > 60 ? url.substring(0, 57) + "…" : url)); return }
            if (!Qt.openUrlExternally(url)) root.fail((r && r.error) || "Could not open the link.")
        })
    }
    function toggleBlock(kind) { root.setBlock(root.blockKind === kind ? "p" : kind) }
    function toggleTask() { root.setBlock(root.blockKind === "task" ? "done" : "task") }
    // Tab nests a list item, Shift+Tab lifts it; in code Tab indents; elsewhere nothing
    // (never out of the editor)
    function editorTab(back) {
        if (/^(ul|ol|task|done)$/.test(root.blockKind)) root.setBlock(back ? "outdent" : "indent")
        // (no-break spaces: inserted plain spaces would collapse; the core writes them as spaces)
        else if (root.blockKind === "code" && !back) richArea.insert(richArea.cursorPosition, "\u00a0\u00a0\u00a0\u00a0")
        // in text and headings an indent, as ddocs.new does (no-break spaces: Qt collapses
        // other inserted spaces, em spaces too)
        else if (root.blockKind !== "code" && root.blockKind !== "table") {
            var c = richArea.cursorPosition, ind = "\u00a0\u00a0\u00a0\u00a0"
            if (!back) richArea.insert(c, ind)
            else if (c >= 4 && richArea.getText(c - 4, c) === ind) richArea.remove(c - 4, c)
        }
    }
    function focusEditor() {
        var a = root.visualEdit ? richArea : mdArea
        a.forceActiveFocus()
        a.cursorPosition = 0
    }
    // The style of the next typed text: { props: { bold: true, … }, pos, prop, value }
    // (Ctrl+B/I with nothing selected, and plain text again after a typed **bold**)
    property var pendingFont: null
    function applyPendingFont() {
        var p = root.pendingFont
        if (!p) return
        var c = richArea.cursorPosition
        if (c <= p.pos || c > p.pos + 8) { if (c !== p.pos) root.pendingFont = null; return }
        root.pendingFont = null
        richArea.select(p.pos, c)
        var f = richArea.cursorSelection.font
        for (var k in p.props) f[k] = p.props[k]
        richArea.cursorSelection.font = f
        richArea.cursorPosition = c
    }
    // Typed Markdown emphasis becomes formatting as the closing marker is typed, with
    // ddocs.new's CommonMark rules (no space just inside the markers: 5 * 3 * 2 stays text).
    readonly property var inlineRules: [
        { re: /(?:^|\s)(\*\*(?!\s+\*\*)([^*\s](?:[^*]*[^*\s])?)\*\*(?!\s+\*\*))$/, tag: "b", off: { bold: false } },
        { re: /(?:^|\s)(__(?!\s+__)([^_\s](?:[^_]*[^_\s])?)__(?!\s+__))$/, tag: "b", off: { bold: false } },
        { re: /(?:^|\s)(\*(?!\s+\*)([^*\s](?:[^*]*[^*\s])?)\*(?!\s+\*))$/, tag: "i", off: { italic: false } },
        { re: /(?:^|\s)(_(?!\s+_)([^_\s](?:[^_]*[^_\s])?)_(?!\s+_))$/, tag: "i", off: { italic: false } },
        { re: /(?:^|\s)(~~(?!\s+~~)([^~\s](?:[^~]*[^~\s])?)~~(?!\s+~~))$/, tag: "s", off: { strikeout: false } },
        { re: /(?:^|[^`])(`([^`\s](?:[^`]*[^`\s])?)`)$/, tag: "code", off: null }
    ]
    property bool applyingRule: false
    function inlineRule() {
        if (root.applyingRule || root.loadingEditor || !richArea.activeFocus) return
        if (root.blockKind === "code" || richArea.selectionStart !== richArea.selectionEnd) return
        var pos = richArea.cursorPosition, p = root.textBefore(pos)
        // (a line the editor restyled while empty starts with a zero-width space)
        var line = p.substring(root.lastBreak(p) + 1).replace(/^[\u200b\ufeff]+/, "")
        for (var i = 0; i < root.inlineRules.length; i++) {
            var r = root.inlineRules[i], m = line.match(r.re)
            if (!m) continue
            var start = pos - m[1].length
            root.applyingRule = true
            richArea.remove(start, pos)
            var isCode = r.tag === "code"
            // after code a zero-width space outside the span, so typing goes on plain (the
            // background cannot be switched off for the next typed text; the core drops it)
            richArea.insert(start, isCode ? root.codeSpan(root.escHtml(m[2])) + "\u200b" : "<" + r.tag + ">" + root.escHtml(m[2]) + "</" + r.tag + ">")
            var end = start + m[2].length + (isCode ? 1 : 0)
            richArea.cursorPosition = end
            root.applyingRule = false
            // what is typed next is plain again
            if (r.off) root.pendingFont = { props: r.off, pos: end, prop: "", value: false }
            return
        }
    }
    function toggleFont(prop) {
        root.commitStep()
        // with nothing selected: the style of the text typed next, the opposite of the
        // character before the cursor (Qt reports the block's style for an empty selection)
        if (richArea.selectionStart === richArea.selectionEnd) {
            var pos = richArea.cursorPosition, on = false
            if (root.pendingFont && root.pendingFont.pos === pos && root.pendingFont.prop === prop) on = root.pendingFont.value
            else if (pos > 0) {
                richArea.select(pos - 1, pos)
                on = !!richArea.cursorSelection.font[prop]
                richArea.cursorPosition = pos
            }
            // Qt types with the style of the character before: style the next typed one,
            // and what follows takes it on
            var props = {}; props[prop] = !on
            root.pendingFont = { prop: prop, value: !on, pos: pos, props: props }
            return
        }
        var f = richArea.cursorSelection.font
        f[prop] = !f[prop]
        richArea.cursorSelection.font = f
        root.editorChanged()
    }
    function escHtml(t) { return String(t).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;") }
    // Inline code and links go in as HTML, which TextArea.insert parses in RichText mode.
    function wrapSelection(make) {
        root.commitStep()
        var s = richArea.selectionStart, e = richArea.selectionEnd
        if (s === e) { root.say("Select some text first"); return false }
        var t = richArea.selectedText
        richArea.remove(s, e)
        richArea.insert(s, make(root.escHtml(t)))
        richArea.forceActiveFocus()
        root.editorChanged()
        return true
    }
    property var linkRange: null
    function askLink() {
        if (richArea.selectionStart === richArea.selectionEnd) { root.say("Select the text to link first"); return }
        root.linkRange = [richArea.selectionStart, richArea.selectionEnd]
        linkUrl.text = "https://"
        linkUrl.forceActiveFocus(); linkUrl.selectAll()
    }
    function findEditor() { return richArea }
    function applyLinkTo(u) { linkUrl.text = u; root.applyLink() }
    function applyLink() {
        var u = linkUrl.text.trim(), r = root.linkRange
        root.linkRange = null
        if (!r || !/^(https?:\/\/|mailto:)\S+$/.test(u)) { if (u !== "" && u !== "https://") root.say("Links must start with https://, http:// or mailto:"); return }
        richArea.select(r[0], r[1])
        root.wrapSelection(function (t) { return root.tintLinks("<a href=\"" + root.escHtml(u) + "\">" + t + "</a>") })
    }
    // Markdown typed at the start of a block becomes that block, as in Fileverse:
    // "# " heading, "- " list, "1. " numbered, "[] " task, "> " quote, "``` " code.
    function typedShortcut() {
        if (root.blockKind === "code" || root.blockKind === "table") return false
        var pos = richArea.cursorPosition
        var p = root.textBefore(pos)
        var raw = p.substring(root.lastBreak(p) + 1)
        // a line the editor restyled while empty holds a zero-width space (Qt drops empty
        // headings): "- " typed there is still "- "
        var typed = raw.replace(/[\u200b\ufeff]/g, "")
        var map = { "#": "h1", "##": "h2", "###": "h3", "-": "ul", "*": "ul", "1.": "ol", "[]": "task", ">": "quote", "```": "code" }
        if (!(typed in map)) return false
        richArea.remove(pos - raw.length, pos)
        root.setBlock(map[typed])
        return true
    }

    function publishDraft() {
        root.saveDraft(function () {
            root.startJob("publish", [root.draft.id], "publish", function (r) { root.refreshNode();
                var where = []
                if (r.ipfs) where.push("IPFS (" + r.ipfs + ")")
                if (r.storage) where.push("Logos Storage")
                root.say("Published to " + where.join(" and ") + (r.notes && r.notes.length ? " - " + r.notes.join("; ") : ""))
                root.openDraft(root.draft.id)
            })
        })
    }

    // ── live documents: while one is open, look for a newer version on chain ──
    // Quiet jobs: no "Opening…" state, no error banner; they only act on success.
    property var quietJobs: ({})          // id -> callback(result)
    function quietJob(method, args, onDone, onFail) {
        root.core(method, args, function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok || !r.jobId) { if (onFail) onFail(r ? r.error : ""); return }
            var q = root.quietJobs; q[r.jobId] = { done: onDone, fail: onFail }; root.quietJobs = q
        })
    }
    function pollQuiet() {
        Object.keys(root.quietJobs).forEach(function (id) {
            root.core("job", [id], function (raw) {
                var r = root.j(raw)
                if (r && r.ok && r.state === "pending") return
                var cb = root.quietJobs[id]
                var q = root.quietJobs; delete q[id]; root.quietJobs = q
                if (r && r.ok && r.state === "done") { if (cb && cb.done) cb.done(r.result) }
                else if (cb && cb.fail) cb.fail(r ? r.error : "")
            })
        })
    }
    Timer { interval: 1500; running: Object.keys(root.quietJobs).length > 0; repeat: true; onTriggered: root.pollQuiet() }
    property bool checkingUpdate: false
    function checkForUpdate() {
        var u = root.docUrl
        if (root.mode !== "doc" || u === "" || u.indexOf("fvbc://") === 0 || root.busy || root.checkingUpdate || root.presenting) return
        root.checkingUpdate = true
        root.quietJob("check_update", [u], function (r) {
            if (!r.changed || root.docUrl !== u) { root.checkingUpdate = false; return }
            root.quietJob("open", [u], function (doc) {
                root.checkingUpdate = false
                if (root.docUrl !== u || root.mode !== "doc") return
                var tab = root.tabIndex, sh = root.sheetIndex
                root.docv = doc
                root.tabIndex = Math.min(tab, Math.max(0, (doc.tabs || []).length - 1))
                root.sheetIndex = Math.min(sh, Math.max(0, (doc.sheets || []).length - 1))
                root.refresh()
                root.say("Updated to the latest version")
            })
        })
        // never stay stuck if a quiet job is lost
        updateGuard.restart()
    }
    Timer { id: updateGuard; interval: 120000; onTriggered: root.checkingUpdate = false }
    Timer { interval: 60000; running: root.mode === "doc"; repeat: true; onTriggered: root.checkForUpdate() }

    Timer { interval: 2500; running: true; repeat: true; onTriggered: root.refresh() }
    Timer { interval: 700; running: root.busy; repeat: true; onTriggered: root.pollJobs() }
    Timer { id: toastTimer; interval: 3500; onTriggered: root.toast = "" }
    // Live preview + autosave while typing (local-first: a draft is saved on this device as you go).
    // a shared draft in live speed saves (and so sends) sooner after a keystroke
    Timer { id: previewTimer; interval: root.isShared && root.collabLive ? 200 : 700; onTriggered: root.saveDraft() }

    property bool snapBusy: false
    property bool snapAgain: false
    function refresh() {
        if (root.snapBusy) { root.snapAgain = true; return }
        root.snapBusy = true
        root.core("snapshot", [], function (raw) {
            var s = root.j(raw)
            if (s && s.ok) { root.snap = s; root.missedPolls = 0; root.coreReachable = true; root.checkOutside() }
            else if (++root.missedPolls >= 3) root.coreReachable = false
            root.snapBusy = false
            if (root.snapAgain) { root.snapAgain = false; root.refresh() }
        })
    }

    Connections {
        target: typeof logos !== "undefined" ? logos : null
        ignoreUnknownSignals: true
        function onModuleEventReceived(moduleName, eventName, data) {
            if (moduleName === root.coreName && eventName === "job_changed") root.pollJobs()
        }
    }
    Component.onCompleted: {
        if (typeof logos !== "undefined" && logos && logos.onModuleEvent) logos.onModuleEvent(root.coreName, "job_changed")
        root.refresh(); root.refreshNode(); root.refreshStorage()
    }

    // Off-screen editor used only to put text on the clipboard.
    TextEdit { id: clip; visible: false; textFormat: TextEdit.PlainText }
    function copy(text, what) { clip.text = text; clip.selectAll(); clip.copy(); root.say((what || "Text") + " copied") }

    FileDialog {
        id: imageDialog
        title: "Choose a picture"
        nameFilters: ["Pictures (*.png *.jpg *.jpeg *.gif *.webp *.svg)", "All files (*)"]
        onAccepted: root.insertImage(selectedFile)
    }
    // "Download Markdown": like a browser download, straight into the Downloads folder
    function downloadMarkdown(title, md) {
        md = String(md || "")
        if (md.length && md.charAt(md.length - 1) !== "\n") md += "\n"
        root.core("download_file", [String(title || ""), "md", md], function (raw) {
            var r = root.j(raw)
            if (r && r.ok) root.say("Downloaded to " + r.path)
            else root.fail("Could not save the file: " + ((r && r.error) || "no answer from the core"))
        })
    }
    // ── Write with AI (AI Gateway's "ai.ask" intent) ─────────────────────────────────
    // Basecamp asks the user first: AI Gateway shows exactly what is sent and lets them
    // pick the model and its privacy. Nothing is written into the document until the user
    // chooses Replace or Insert. Without AI Gateway the request answers "unavailable" and
    // Basecamp offers to install it, so Fileverse needs no dependency on it.
    property bool aiOpen: false
    property bool aiBusy: false
    property string aiNote: ""           // stays under the AI actions (e.g. AI Gateway missing)
    property string aiField: ""          // "", "ask" or "translate": the row's text field is for that
    property var aiResult: null           // { text, model, privacy, connector, label, how, a, b, orig, visual }
    readonly property int aiMaxBytes: 60000   // Basecamp's limit is 64 KB a string
    readonly property string aiSystem: "You help someone write a document in a Markdown editor. Answer in Markdown, in the language of the text unless asked otherwise. Reply with only the text asked for: no introduction, no commentary, no code fences around the answer."
    readonly property var aiActions: [
        // label, prompt, what it works on, how the answer goes in
        ["Continue writing", "Continue the text from where it ends: write the next paragraph or two in the same language, voice and formatting. Reply with only the new text.", "before", "cursor"],
        ["Clearer", "Rewrite this text to be clearer and easier to read. Keep its meaning, language and Markdown formatting.", "target", "replace"],
        ["Shorter", "Make this text shorter. Keep the important points, its language and Markdown formatting.", "target", "replace"],
        ["More formal", "Rewrite this text in a more formal tone. Keep its meaning, language and Markdown formatting.", "target", "replace"],
        ["Fix grammar", "Fix spelling, grammar and punctuation in this text. Change nothing else; keep the Markdown formatting.", "target", "replace"],
        ["Summarize", "Summarize this text in a few sentences.", "targetOrAll", "below"]
    ]
    readonly property string aiTranslateDefault: "Translate this text into {language}. Keep the Markdown formatting."
    // The user's own prompts (Settings → AI prompts), kept in this device's preferences only:
    // { system, translate, chat, actions: { label: prompt }, mine: [{ name, prompt, how }] }.
    // An empty or missing entry means Fileverse's default.
    // aiPSaved: what was last sent to the core, so quick edits in a row build on each other
    property var aiPSaved: null
    readonly property var aiP: root.aiPSaved || (root.snap && root.snap.prefs && root.snap.prefs.aiPrompts) || ({})
    function aiOwn(key) { var v = root.aiP[key]; return typeof v === "string" && v.trim() !== "" ? v : "" }
    readonly property string aiSystemNow: root.aiOwn("system") || root.aiSystem
    readonly property string aiTranslateNow: root.aiOwn("translate") || root.aiTranslateDefault
    readonly property var aiMine: Array.isArray(root.aiP.mine) ? root.aiP.mine : []
    readonly property var aiActionsNow: root.aiActions.map(function (a) {
        var own = root.aiP.actions && typeof root.aiP.actions[a[0]] === "string" && root.aiP.actions[a[0]].trim() !== "" ? root.aiP.actions[a[0]] : ""
        return [a[0], own || a[1], a[2], a[3]]
    }).concat(root.aiMine.map(function (m) {
        return [m.name, m.prompt, m.how === "replace" ? "target" : m.how === "cursor" ? "before" : "targetOrAll", m.how === "cursor" ? "cursor" : m.how === "replace" ? "replace" : "below"]
    }))
    // the prompts Settings lists: key, title, Fileverse's default
    readonly property var aiPromptList: [["system", "Write with AI: how to answer", root.aiSystem]]
        .concat(root.aiActions.map(function (a) { return ["a:" + a[0], a[0], a[1]] }))
        .concat([["translate", "Translate ({language} is the language you type)", root.aiTranslateDefault],
                 ["chat", "AI chat: who the AI is and how it talks", root.chatRole]])
    function aiPromptNow(key, def) {
        if (key.indexOf("a:") === 0) { var acts = root.aiP.actions || {}; var v = acts[key.substring(2)]; return typeof v === "string" && v.trim() !== "" ? v : def }
        return root.aiOwn(key) || def
    }
    function aiPromptEdited(key, def) { return root.aiPromptNow(key, def) !== def }
    function aiSavePrompts(p, msg) {
        root.aiPSaved = p
        root.core("set_pref", ["aiPrompts", JSON.stringify(p)], function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok) { root.aiPSaved = null; root.fail((r && r.error) || "Could not save the prompt"); return }
            var sn = JSON.parse(JSON.stringify(root.snap)); sn.prefs = r.prefs; root.snap = sn
            root.say(msg)
        })
    }
    // text "" (or the default) = back to Fileverse's prompt
    function aiSetPrompt(key, def, text) {
        var p = JSON.parse(JSON.stringify(root.aiP))
        var t = String(text || "")
        var own = t.trim() !== "" && t !== def
        if (key.indexOf("a:") === 0) {
            p.actions = p.actions || {}
            if (own) p.actions[key.substring(2)] = t; else delete p.actions[key.substring(2)]
        } else if (own) p[key] = t
        else delete p[key]
        root.aiSavePrompts(p, own ? "Prompt saved on this device" + (key === "chat" && root.chatSession ? ". It applies from the next AI chat you open" : "") : "Fileverse's prompt restored")
    }
    // index -1 adds; prompt "" removes
    function aiSetMine(index, name, prompt, how) {
        var p = JSON.parse(JSON.stringify(root.aiP))
        var mine = Array.isArray(p.mine) ? p.mine : []
        if (prompt === "") { mine.splice(index, 1); p.mine = mine; root.aiSavePrompts(p, "Action removed"); return true }
        name = String(name).trim(); prompt = String(prompt)
        if (!name || !prompt.trim()) { root.fail("An action needs a name and a prompt."); return false }
        var taken = root.aiActions.some(function (a) { return a[0].toLowerCase() === name.toLowerCase() })
                    || ["translate…", "ask ai…"].indexOf(name.toLowerCase()) >= 0
                    || mine.some(function (m, i) { return i !== index && m.name.toLowerCase() === name.toLowerCase() })
        if (taken) { root.fail("There is already an action called " + name + "."); return false }
        var m = { name: name.substring(0, 40), prompt: prompt, how: how }
        if (index < 0) { if (mine.length >= 20) { root.fail("At most 20 actions of your own."); return false } mine.push(m) } else mine[index] = m
        p.mine = mine
        root.aiSavePrompts(p, index < 0 ? "Action added to Write with AI" : "Action saved")
        return true
    }
    // The prompt being edited in the dialog: { key, title, def, text } or, for an action
    // of your own, { mine: index (-1 new), name, text, how }.
    property var promptEdit: null
    property string promptHow: "replace"
    function utf8Len(t) { return unescape(encodeURIComponent(t)).length }
    // At most aiMaxBytes of `t`: its end (`keepEnd`, for "continue") or its start.
    function aiCap(t, keepEnd) {
        t = String(t || "")
        if (root.utf8Len(t) <= root.aiMaxBytes) return t
        var lo = 0, hi = t.length
        while (lo < hi) {       // the longest piece that fits
            var mid = Math.ceil((lo + hi) / 2)
            var piece = keepEnd ? t.substring(t.length - mid) : t.substring(0, mid)
            if (root.utf8Len(piece) <= root.aiMaxBytes) lo = mid; else hi = mid - 1
        }
        return keepEnd ? t.substring(t.length - lo) : t.substring(0, lo)
    }
    // The editor's plain text (what positions count) and Markdown of a range.
    function aiPlain() { return root.visualEdit ? richArea.getText(0, richArea.length) : mdArea.text }
    function aiMarkdown(a, b) {
        if (!root.visualEdit) return mdArea.text.substring(a, b)
        if (a >= b) return ""
        var r = root.coreSync("rich_md", [root.draft.id, richArea.getFormattedText(a, b)])
        return (r && r.markdown !== undefined) ? r.markdown : richArea.getText(a, b)
    }
    // What an action works on: the selection, else the paragraph the cursor is in.
    function aiTarget() {
        var ed = root.visualEdit ? richArea : mdArea
        var a = ed.selectionStart, b = ed.selectionEnd
        if (a !== b) return { a: a, b: b, what: "the selection" }
        var t = root.aiPlain(), c = ed.cursorPosition
        if (root.visualEdit) {
            a = c; while (a > 0 && !root.pieceBreak.test(t.charAt(a - 1))) a--
            b = c; while (b < t.length && !root.pieceBreak.test(t.charAt(b))) b++
        } else {
            a = t.lastIndexOf("\n\n", c - 1); a = a < 0 ? 0 : a + 2
            b = t.indexOf("\n\n", c); if (b < 0) b = t.length
        }
        return { a: a, b: b, what: "this paragraph" }
    }
    function aiRun(label, prompt, on, how) {
        if (!root.draft || root.aiBusy) return
        if (typeof logos === "undefined" || logos === null || typeof logos.request !== "function") {
            root.fail("This Basecamp cannot ask other apps yet. Writing with AI needs Basecamp 0.3.2 or newer and the AI Gateway app.")
            return
        }
        var ed = root.visualEdit ? richArea : mdArea
        var t, content
        if (on === "before") {
            t = { a: ed.cursorPosition, b: ed.cursorPosition, what: "the text before the cursor" }
            content = root.aiCap(root.aiMarkdown(0, ed.cursorPosition), true)
        } else {
            // the answer goes with the selection or the cursor's paragraph; Summarize and Ask
            // read the whole document when nothing is selected
            t = root.aiTarget()
            var whole = on === "targetOrAll" && ed.selectionStart === ed.selectionEnd
            content = root.aiCap(whole ? root.aiMarkdown(0, ed.length) : root.aiMarkdown(t.a, t.b), false)
        }
        if (!String(content).trim()) { root.fail("There is no text to work on: select some, or put the cursor in a paragraph."); return }
        var plain = root.aiPlain()
        var job = { label: label, how: how, a: t.a, b: t.b, orig: plain.substring(t.a, t.b), visual: root.visualEdit, draftId: root.draft.id }
        root.commitStep()
        root.aiBusy = true
        root.aiNote = ""
        root.aiResult = null
        logos.request("ai.ask", { prompt: root.aiCap(prompt, false), content: content, system: root.aiSystemNow }, function (res) {
            root.aiBusy = false
            if (typeof res === "string") res = root.j(res)
            if (!res || !res.ok) {
                var e = res ? String(res.error || "failed") : "failed"
                if (e === "unavailable") root.aiNote = "Writing with AI needs the AI Gateway app. Install it in Basecamp, then try again."
                else if (e === "cancelled") root.say("Not sent to the AI")
                else if (e === "timeout") root.fail("The AI did not answer in time. Try again, or pick a faster model in AI Gateway.")
                else root.fail("The AI could not answer (" + e + ")" + (res && res.message ? ": " + res.message : "") + ".")
                return
            }
            if (!root.draft || root.draft.id !== job.draftId) return
            var d = res.data || {}
            job.text = root.aiClean(String(d.text || ""))
            job.model = d.model || ""; job.privacy = d.privacy || ""; job.connector = d.connector || ""
            if (!job.text.trim()) { root.fail("The AI answered with no text."); return }
            root.aiResult = job
        })
    }
    // Models like to wrap answers in a code fence; the document wants the text.
    function aiClean(t) {
        var m = t.trim().match(/^```[a-zA-Z]*\n([\s\S]*)\n```$/)
        return m ? m[1] : t.trim()
    }
    // Whether the text the AI worked on is still where it was.
    function aiRangeIntact(r) {
        return r && r.visual === root.visualEdit && root.aiPlain().substring(r.a, r.b) === r.orig
    }
    // Put the answer in: "replace" the text it worked on, "below" it as new paragraphs,
    // or at the "cursor". One undo step; shared documents send it like typing.
    // The answer panel closes first and the text changes once the editor has its new size:
    // changed while the panel was still open, Qt drew only the part of the text then in view,
    // and the part the closing panel uncovered stayed blank (vpavlin, a Replace on a long
    // answer; scrolling did not help, a redo did).
    function aiApply(how) {
        var r = root.aiResult
        if (!r || !root.draft) return
        if (how !== "cursor" && !root.aiRangeIntact(r)) { root.fail("The text changed while the AI was writing. Use Insert at cursor or Copy instead."); return }
        root.aiResult = null
        aiApplyLater.job = { r: r, how: how }
        aiApplyLater.restart()
    }
    Timer {
        id: aiApplyLater
        property var job: null
        interval: 60
        onTriggered: if (job && root.draft && root.draft.id === job.r.draftId) root.aiApplyNow(job.r, job.how)
    }
    function aiApplyNow(r, how) {
        var ed = root.visualEdit ? richArea : mdArea
        if (how !== "cursor" && !root.aiRangeIntact(r)) { root.fail("The text changed while the AI was writing. Use Insert at cursor or Copy instead."); return }
        var md = r.text
        root.commitStep()
        var at = how === "cursor" ? ed.cursorPosition : how === "below" ? r.b : r.a
        if (root.visualEdit) {
            var htmlBefore = richArea.text, len0 = richArea.length
            // Whole paragraphs (Replace of whole blocks, Insert below): build the new Markdown
            // from the text before, the answer and the text after, and load it, as opening
            // does. Qt's fragment insert merged the answer's first paragraph into the block
            // left at the cut (a list item) and left paragraphs it did not draw.
            var pl = root.aiPlain()
            var whole = how === "below" || (how === "replace"
                        && (r.a === 0 || root.pieceBreak.test(pl.charAt(r.a - 1)))
                        && (r.b >= pl.length || root.pieceBreak.test(pl.charAt(r.b))))
            if (whole) {
                // the Markdown on either side, without the paragraph break at the cut (it came
                // back as an empty paragraph, "&nbsp;")
                var preEnd = how === "below" ? r.b : r.a - 1
                var pre = preEnd > 0 ? root.aiMarkdown(0, preEnd).replace(/\s+$/, "") : ""
                var post = r.b + 1 < richArea.length ? root.aiMarkdown(r.b + 1, richArea.length).replace(/^\s+/, "") : ""
                var next = (pre ? pre + "\n\n" : "") + md.trim() + (post ? "\n\n" + post : "") + "\n"
                root.loadEditor(next)
                var where = root.cursorFor(richArea.getText(0, richArea.length), md.trim().split("\n").pop().replace(/^\s*([-*+]|\d+[.)]|#+|>)\s+/, "").replace(/[*_`~]/g, ""), 0.5)
                richArea.cursorPosition = Math.min(where, richArea.length)
                root.histBase = htmlBefore; root.stepOpen = true; root.stepCaret = at
                root.commitStep()
                richArea.forceActiveFocus()
                root.editorChanged()
                root.commitStep()
                root.aiResult = null
                root.say("Added the AI's text. Undo (Ctrl+Z) takes it out again.")
                return
            }
            // Inside a paragraph: Qt's fragment insert joins the answer to the text around it;
            // then the editor is rebuilt from the Markdown (Qt can leave paragraphs it does
            // not draw), and the whole change is one undo step from the text before it.
            root.loadingEditor = true
            if (how === "replace") richArea.remove(r.a, r.b)
        } else if (how === "replace") ed.remove(r.a, r.b)
        if (root.visualEdit) {
            var h = root.coreSync("rich_html", [md, root.draft.id, root.viewDir])
            if (!h || !h.ok) { richArea.text = htmlBefore; root.loadingEditor = false; root.fail("Could not put the answer in the editor. Use Copy instead."); return }
            var body = String(h.html).replace(/^[\s\S]*?<body[^>]*>/i, "").replace(/<\/body>[\s\S]*$/i, "")
            // a fragment's first block joins the block it lands in, and Qt drops an empty one:
            // a zero-width space (which the core drops when it saves) starts the new block below
            if (how === "below") body = "<p>\u200b</p>" + body
            // a space between the text before the cursor and the answer (Qt drops a plain leading space)
            else if (how === "cursor" && at > 0 && !/\s/.test(root.aiPlain().charAt(at - 1))) body = body.replace(/^(\s*<p[^>]*>)/i, '$1<span style="white-space:pre"> </span>')
            richArea.insert(at, body)
            var caret = Math.max(0, at + richArea.length - (how === "replace" ? len0 - (r.b - r.a) : len0))
            root.loadingEditor = false
            root.loadEditor(root.editorMarkdown())
            richArea.cursorPosition = Math.min(caret, richArea.length)
            root.histBase = htmlBefore; root.stepOpen = true; root.stepCaret = at
            root.commitStep()
            richArea.forceActiveFocus()
        } else {
            var text = how === "below" ? "\n\n" + md : how === "cursor" && at > 0 && !/\s/.test(mdArea.text.charAt(at - 1)) ? " " + md : md
            mdArea.insert(at, text)
            mdArea.forceActiveFocus()
        }
        root.editorChanged()
        root.commitStep()
        root.aiResult = null
        root.say("Added the AI's text. Undo (Ctrl+Z) takes it out again.")
    }
    // ── AI chat (AI Gateway's "ai.chat" intent) ─────────────────────────────────────
    // A conversation about the open document, kept on this device only (core chat_load /
    // chat_save). ai.chat keeps no state between requests, so each one carries the
    // conversation; the document goes in only when it changed since the AI last saw it:
    // as a full copy (older copies are then left out) or, for a small change, as the
    // changed lines. The AI proposes edits as FIND/REPLACE blocks; each shows as a card
    // the user applies or dismisses. Nothing changes in the document by itself.
    property bool chatOpen: false
    property bool chatBusy: false
    property string chatDraftId: ""
    property var chatMsgs: []        // { role: user|assistant|doc|note, text, raw?, edits?, full?, omitted?, model?, privacy? }
    property string chatDocSent: ""  // the document as the AI last saw it
    property string chatNote: ""
    readonly property int chatMaxBytes: 60000
    readonly property string chatRole: "You are a co-author in Fileverse, a Markdown document editor. You and the user talk about the document they are writing. Answer in the user's language, briefly and to the point."
    // what the app needs from the AI (where the document is, the edit format); not editable
    readonly property string chatSystem: root.chatRoleNow + "\n\nThe document's Markdown is in the messages that start with [Document]; messages that start with [Document changes] say what changed since.\n\nWhen the user asks you to change the document, or a change clearly helps, propose it with blocks of exactly this form, one block per change:\n\n```edit\n<<<<<<< FIND\ntext copied exactly from the current document\n=======\nthe new text\n>>>>>>> REPLACE\n```\n\nFIND must be copied character for character from the current Markdown, with its ** and other marks and its &nbsp; lines (empty paragraphs), never shortened with ..., and occur in it only once: a few whole lines, short but unique. For several changes far apart, use several blocks rather than one large one. To add text, FIND a nearby line and repeat it in the new text together with the addition. To delete, leave the new text empty. The user reviews every block before it is applied. Outside the blocks, talk normally and do not repeat the blocks in prose."
    readonly property string chatRoleNow: root.aiOwn("chat") || root.chatRole
    function chatSave() {
        if (!root.chatDraftId) return
        var c = root.chatMsgs.length ? JSON.stringify({ v: 1, msgs: root.chatMsgs, docSent: root.chatDocSent }) : "null"
        root.core("chat_save", [root.chatDraftId, c], function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok) root.chatNote = "Could not keep the chat on this device: " + ((r && r.error) || "no answer from the core")
        })
    }
    function chatLoad() {
        if (!root.draft) return
        var id = root.draft.id
        root.chatDraftId = id
        root.chatMsgs = []; root.chatDocSent = ""; root.chatNote = ""
        root.core("chat_load", [id], function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok || root.chatDraftId !== id) return
            var c = r.chat
            if (c && c.msgs) { root.chatMsgs = c.msgs; root.chatDocSent = c.docSent || "" }
        })
    }
    onDraftChanged: if (root.chatOpen && root.draft && root.draft.id !== root.chatDraftId) { root.chatCloseSession(); root.chatLoad() }
    function chatToggle() {
        root.chatOpen = !root.chatOpen
        if (!root.chatOpen) root.chatCloseSession()
        else if (root.draft && root.draft.id !== root.chatDraftId) root.chatLoad()
    }
    function chatClear() {
        // the approved session stays; its next turn tells AI Gateway to forget the conversation
        if (root.chatSession) { var sess = Object.assign({}, root.chatSession); sess.reset = true; root.chatSession = sess }
        root.chatMsgs = []; root.chatDocSent = ""; root.chatNote = ""
        root.chatSave()
    }
    // The document as the AI sees it: the title as a heading, then the text.
    function chatDocument() { return root.withTitle(titleField.text, root.editorMarkdown()) }
    // What changed between two versions, by lines: the first and last differing lines and
    // what is between them (one region; enough for the usual edit between two messages).
    function chatDiff(a, b) {
        var x = a.split("\n"), y = b.split("\n"), p = 0
        while (p < x.length && p < y.length && x[p] === y[p]) p++
        var q = 0
        while (q < x.length - p && q < y.length - p && x[x.length - 1 - q] === y[y.length - 1 - q]) q++
        var was = x.slice(p, x.length - q), now = y.slice(p, y.length - q)
        var where = p === 0 ? "At the start of the document" : "After the line " + JSON.stringify(x[p - 1].substring(0, 120))
        if (!was.length) return where + ", these lines were added:\n```\n" + now.join("\n") + "\n```"
        if (!now.length) return where + ", these lines were removed:\n```\n" + was.join("\n") + "\n```"
        return where + ", these lines:\n```\n" + was.join("\n") + "\n```\nare now:\n```\n" + now.join("\n") + "\n```"
    }
    // The request: the instructions, then the conversation with each document message as
    // the AI should read it; neighbouring user messages are joined (some models want turns
    // to alternate). Oldest turns go first when it would not fit.
    function chatRequest(msgs) {
        var title = titleField.text.trim() || "Untitled"
        var turns = []
        msgs.forEach(function (m) {
            var role = m.role === "assistant" ? "assistant" : "user", content
            if (m.role === "note") return
            if (m.role === "doc") content = m.omitted ? "[An earlier copy of the document was here; a newer full copy follows.]"
                                       : m.full ? "[Document: " + JSON.stringify(title) + "]\n" + m.text : "[Document changes]\n" + m.text
            else content = m.role === "assistant" ? (m.raw || m.text) : m.text
            var last = turns[turns.length - 1]
            if (last && last.role === role) last.content += "\n\n" + content
            else turns.push({ role: role, content: content })
        })
        var size = function () { return turns.reduce(function (n, t) { return n + root.utf8Len(t.content) + 32 }, root.utf8Len(root.chatSystem)) }
        // keep the turn holding the latest full copy of the document and everything after it
        var keepFrom = 0
        for (var k = turns.length - 1; k >= 0; k--) if (turns[k].content.indexOf("[Document: ") >= 0) { keepFrom = k; break }
        while (size() > root.chatMaxBytes && turns.length > 1 && keepFrom > 0) { turns.shift(); keepFrom-- }
        if (size() > root.chatMaxBytes) turns.forEach(function (t) { t.content = root.aiCap(t.content, false) })
        while (turns.length && turns[0].role === "assistant") turns.shift()
        return [{ role: "system", content: root.chatSystem }].concat(turns)
    }
    // The AI's reply: its words and its edit blocks.
    function chatParse(t) {
        var edits = [], re = /```edit[^\n]*\n<<<<<<< FIND\n([\s\S]*?)\n=======\n([\s\S]*?)\n?>>>>>>> REPLACE[ \t]*\n?```/g, m
        while ((m = re.exec(t)) !== null) edits.push({ find: m[1], replace: m[2], state: "new" })
        var prose = t.replace(re, "").replace(/\n{3,}/g, "\n\n").trim()
        return { text: prose, edits: edits }
    }
    // Without AI Gateway sessions (an older AI Gateway): every message is an ai.chat request
    // that Basecamp confirms, carrying the conversation and the document's changes.
    function chatProse() {
        if (!root.chatSession) return
        var sess = Object.assign({}, root.chatSession); sess.prose = true; root.chatSession = sess
    }
    // A structured reply: { reply, edits: [{ find, replace, why }] }, possibly in a code fence.
    // null when it is not that.
    function chatParseJson(t) {
        var s = String(t || "").trim().replace(/^```(?:json)?\s*\n([\s\S]*)\n```$/, "$1")
        var o
        try { o = JSON.parse(s) } catch (e) { return null }
        if (!o || typeof o !== "object" || typeof o.reply !== "string" || !Array.isArray(o.edits)) return null
        var edits = o.edits.filter(function (e) { return e && typeof e.find === "string" && typeof e.replace === "string" })
                           .map(function (e) { return { find: e.find, replace: e.replace, why: typeof e.why === "string" ? e.why : "", state: "new" } })
        // what the model sees of its own turn later (AI Gateway keeps the JSON; this is for the
        // local history and the prose fallback): its words and the edits as blocks
        var raw = o.reply + edits.map(function (e) { return "\n\n```edit\n<<<<<<< FIND\n" + e.find + "\n=======\n" + e.replace + "\n>>>>>>> REPLACE\n```" }).join("")
        return { text: o.reply.trim(), edits: edits, raw: raw }
    }
    function chatSendStateless(text) {
        text = String(text || "").trim()
        if (!text || root.chatBusy || !root.draft) return false
        if (typeof logos === "undefined" || logos === null || typeof logos.request !== "function") {
            root.chatNote = "This Basecamp cannot ask other apps yet. AI chat needs Basecamp 0.3.2 or newer and the AI Gateway app."
            return false
        }
        root.commitStep()
        var before = { msgs: root.chatMsgs, docSent: root.chatDocSent }
        var msgs = root.chatMsgs.slice(), doc = root.chatDocument()
        if (doc !== root.chatDocSent) {
            var diff = root.chatDocSent ? root.chatDiff(root.chatDocSent, doc) : ""
            if (diff && root.utf8Len(diff) < Math.min(20000, root.utf8Len(doc) / 2)) {
                msgs.push({ role: "doc", full: false, text: diff })
            } else {
                // a new full copy: the AI no longer needs the older ones (or their changes)
                msgs = msgs.map(function (m) { return m.role === "doc" && !m.omitted ? Object.assign({}, m, { omitted: true, text: "" }) : m })
                msgs.push({ role: "doc", full: true, text: root.aiCap(doc, false) })
            }
        }
        msgs.push({ role: "user", text: text })
        root.chatMsgs = msgs; root.chatDocSent = doc; root.chatNote = ""
        root.chatBusy = true
        var id = root.chatDraftId
        logos.request("ai.chat", { messages: root.chatRequest(msgs) }, function (res) {
            root.chatBusy = false
            if (root.chatDraftId !== id) return
            if (typeof res === "string") res = root.j(res)
            if (!res || !res.ok) {
                var e = res ? String(res.error || "failed") : "failed"
                // nothing reached the AI: back to before, the message back in the box
                root.chatMsgs = before.msgs; root.chatDocSent = before.docSent
                chatInput.text = text
                root.chatNote = e === "unavailable" ? "AI chat needs the AI Gateway app. Install it in Basecamp, then try again."
                              : e === "cancelled" ? "Not sent to the AI."
                              : e === "timeout" ? "The AI did not answer in time. Try again, or pick a faster model in AI Gateway."
                              : "The AI could not answer (" + e + ")" + (res && res.message ? ": " + res.message : "") + "."
                return
            }
            var d = res.data || {}, raw = String(d.text || ""), parsed = root.chatParse(raw)
            var m2 = root.chatMsgs.slice()
            m2.push({ role: "assistant", raw: raw, text: parsed.text, edits: parsed.edits, model: d.model || "", privacy: d.privacy || "" })
            root.chatMsgs = m2
            root.chatSave()
        })
        root.chatSave()
        return true
    }
    // ── AI chat through an ai.session intent ──
    // The view asks once (Basecamp and the providing app, e.g. AI Gateway, confirm; the user
    // picks the model); then fileverse_core sends each turn, without a dialog, to the module
    // the intent's answer names (the ai-session/1 contract). Fileverse depends on no provider.
    // The provider keeps the conversation and the latest document; Fileverse sends the new
    // message, and the whole document only when it changed. Without sessions: ai.chat.
    property var chatSession: null        // { id, provider, model, privacy, doc }: doc = what the provider has
    property bool chatStateless: false
    property string chatPartial: ""       // the answer as it streams in
    property var chatDeltaOk: null        // jobDelta: null not tried yet, false refused (older core or AI Gateway)
    property string chatJob: ""
    property var chatPending: null        // { text, doc, before, retried }
    readonly property string chatSessionSystem: root.chatRoleNow + "\n\nThe document's latest Markdown is given below and is updated as the user edits. Each message may come with context saying what the user has selected or where their cursor is: \"this\", \"here\" or \"the selection\" means that text.\n\nWhen the user asks you to change the document, or a change clearly helps, propose edits. Each edit has: find, text copied character for character from the current Markdown (with its ** and other marks and its &nbsp; lines, never shortened with ...), a few whole lines that occur only once; replace, the new text (empty to delete; to add text, find a nearby line and repeat it with the addition); why, one short sentence. Use one edit per change; changes far apart are separate edits. The user reviews every edit before it is applied.\n\nWhen a JSON response format is required, answer with it: your words in reply, the edits in edits (an empty list when there are none). Otherwise write each edit as a block of exactly this form, and talk normally outside the blocks:\n\n```edit\n<<<<<<< FIND\ntext copied exactly from the current document\n=======\nthe new text\n>>>>>>> REPLACE\n```"
    // AI Gateway's structured output (responseFormat json_schema): the reply and the edits as
    // JSON, so edits come exactly as the model meant them. A model that can't, or a turn that
    // fails or does not parse with it, puts the session back on the prose format above.
    readonly property var chatSchema: ({
        type: "json_schema",
        json_schema: { name: "fileverse_chat_reply", strict: true, schema: {
            type: "object", additionalProperties: false, required: ["reply", "edits"],
            properties: {
                reply: { type: "string" },
                edits: { type: "array", items: { type: "object", additionalProperties: false, required: ["find", "replace", "why"],
                         properties: { find: { type: "string" }, replace: { type: "string" }, why: { type: "string" } } } }
            } } }
    })
    // Where the user is, for the turn's `context` (AI Gateway sends it with that turn only).
    function chatContext() {
        var ed = root.visualEdit ? richArea : mdArea
        if (!root.draft) return ""
        var t = root.aiTarget(), md = String(root.aiMarkdown(t.a, t.b) || "").trim()
        if (!md) return ""
        if (md.length > 4000) md = md.substring(0, 4000) + "…"
        return (ed.selectionStart !== ed.selectionEnd ? "The user has selected this text:\n" : "The user's cursor is in this paragraph:\n") + md
    }
    function chatSend(text) {
        text = String(text || "").trim()
        if (!text || root.chatBusy || !root.draft) return false
        if (root.chatStateless) return root.chatSendStateless(text)
        if (typeof logos === "undefined" || logos === null || typeof logos.request !== "function") {
            root.chatNote = "This Basecamp cannot ask other apps yet. AI chat needs Basecamp 0.3.2 or newer and the AI Gateway app."
            return false
        }
        root.commitStep()
        var doc = root.chatDocument()
        var pend = { text: text, doc: doc, before: { msgs: root.chatMsgs, docSent: root.chatDocSent }, retried: false }
        var msgs = root.chatMsgs.slice()
        if (!root.chatSession || root.chatSession.doc !== doc) msgs.push({ role: "doc", full: true, text: "" })
        msgs.push({ role: "user", text: text })
        root.chatMsgs = msgs; root.chatNote = ""; root.chatBusy = true; root.chatPartial = ""
        root.chatPending = pend
        if (root.chatSession) root.chatTurn(false)
        else root.chatOpenSession()
        return true
    }
    // Nothing reached the AI: the chat as it was, the message back in the box.
    function chatUndoSend(note) {
        var p = root.chatPending
        root.chatBusy = false; root.chatPartial = ""; root.chatJob = ""; root.chatPending = null
        if (!p) return
        root.chatMsgs = p.before.msgs; root.chatDocSent = p.before.docSent
        chatInput.text = p.text
        if (note) root.chatNote = note
    }
    function chatFailNote(e, res) {
        var msg = res && (res.message || (res.data && res.data.message)) ? ": " + (res.message || res.data.message) : ""
        return e === "unavailable" ? "AI chat needs the AI Gateway app. Install it in Basecamp, then try again."
             : e === "cancelled" ? "Not sent to the AI" + msg + "."
             : e === "timeout" ? "The AI did not answer in time. Try again, or pick a faster model in AI Gateway."
             : "The AI could not answer (" + e + ")" + msg + "."
    }
    function chatOpenSession() {
        var p = root.chatPending; if (!p) return
        var title = titleField.text.trim() || "Untitled"
        // an intent's strings are at most 64 KB: a longer document goes with the first turn
        var docNow = root.utf8Len(p.doc) <= root.aiMaxBytes ? p.doc : ""
        logos.request("ai.session", { title: title, purpose: "Talk with an AI about this Fileverse document; it proposes edits, and nothing changes until you apply them. Sends: this document's text, your messages and what you select or where your cursor is.",
                                      system: root.chatSessionSystem, document: docNow }, function (res) {
            if (root.chatPending !== p) return
            if (typeof res === "string") res = root.j(res)
            if (!res || !res.ok) {
                var e = res ? String(res.error || "failed") : "failed"
                if (e === "unavailable") {
                    // an AI Gateway without sessions (or none): one ai.chat request per message
                    root.chatUndoSend("")
                    root.chatStateless = true
                    if (root.chatSendStateless(p.text)) chatInput.text = ""
                    return
                }
                root.chatUndoSend(root.chatFailNote(e, res))
                return
            }
            var d = res.data || {}
            // Whichever app answered ai.session runs the chat; its answer names the module to
            // send the turns to and the contract they follow. AI Gateway before 0.1.13 named
            // neither: it was the only provider, and spoke ai-session/1.
            var api = String(d.api || "ai-session/1")
            if (api !== "ai-session/1") {
                root.chatUndoSend("")
                root.chatStateless = true   // a session this view cannot talk to: one ai.chat request per message
                if (root.chatSendStateless(p.text)) chatInput.text = ""
                return
            }
            root.chatSession = { id: String(d.sessionId || ""), provider: String(d.provider || "ai_gateway_core"),
                                 model: d.model || "", privacy: d.privacy || "", connector: d.connector || "", doc: docNow, prose: false, reset: false }
            root.chatTurn(true)
        })
    }
    // One turn: the new message (with the earlier conversation when the session is new),
    // and the document when AI Gateway's copy is not the current one.
    function chatTurn(fresh) {
        var p = root.chatPending, sess = root.chatSession
        if (!p || !sess) return
        p.text = ""; p.since = 0; p.deltaDone = false; p.polling = false   // a new job streams from its start
        var turn = {}
        if (fresh) {
            var past = []
            p.before.msgs.forEach(function (m) {
                if (m.role === "user") past.push({ role: "user", content: m.text })
                else if (m.role === "assistant") past.push({ role: "assistant", content: m.raw || m.text })
            })
            past = past.slice(-60)
            while (past.length && past[0].role !== "user") past.shift()
            turn.messages = past.concat([{ role: "user", content: p.text }])
        } else turn.message = p.text
        if (sess.doc !== p.doc) turn.document = p.doc
        if (sess.reset) turn.resetHistory = true
        if (p.context === undefined) p.context = root.chatContext()
        if (p.context) turn.context = p.context
        p.fresh = fresh
        p.structured = !sess.prose
        if (p.structured) turn.responseFormat = root.chatSchema
        root.aiSessionCall(sess, "sessionChat", [sess.id, JSON.stringify(turn)], function (raw) {
            if (root.chatPending !== p) return
            var r = root.j(raw)
            if (r && r.ok === false && p.structured && /unsupported/.test(String(r.code || r.error || ""))) {
                // the model is known not to do structured output: this session uses prose
                root.chatProse(); root.chatTurn(fresh); return
            }
            if (r && r.absent) {
                // the session's app is not there to talk to: one ai.chat request per message
                root.chatSession = null
                root.chatUndoSend("")
                root.chatStateless = true
                if (root.chatSendStateless(p.text)) chatInput.text = ""
                return
            }
            var code = r ? String(r.code || r.error || "") : ""
            if (r && r.ok === false && /no_session/.test(code) && !p.retried) {
                // closed, idle too long or AI Gateway restarted: ask for a new session (once)
                p.retried = true
                root.chatSession = null
                root.chatOpenSession()
                return
            }
            var job = r ? String(r.jobId || r.id || (r.job && r.job.id) || "") : ""
            if (!r || r.ok === false || !job) {
                root.chatUndoSend(/busy/.test(code) ? "The AI is still answering the last message; wait a moment." : root.chatFailNote(code || "failed", r))
                return
            }
            root.chatJob = job
            chatPoll.start()
        })
    }
    Timer {
        id: chatPoll
        interval: 350; repeat: true   // the answer streams in: AI Gateway's job text grows as it arrives
        onTriggered: {
            var p = root.chatPending, job = root.chatJob
            if (!p || !job) { stop(); return }
            if (p.polling) return   // the last call has not answered yet
            p.polling = true
            // While the answer runs: only the new text (AI Gateway 0.1.10's jobDelta, which the
            // core allows from 0.17.2). An older core or gateway refuses it: job() from then on.
            if (root.chatDeltaOk !== false && !p.deltaDone) {
                root.aiSessionCall(root.chatSession, "jobDelta", [job, String(p.since || 0)], function (raw) {
                    p.polling = false
                    if (root.chatPending !== p || root.chatJob !== job) return
                    var d = root.j(raw) || {}
                    if (d.state === undefined || d.state === null || d.state === "") { root.chatDeltaOk = false; return }
                    root.chatDeltaOk = true
                    if (typeof d.delta === "string") {
                        // `length` counts UTF-8 bytes: only ever handed back, never compared with JS lengths
                        if (String(d.offset || 0) === String(p.since || 0)) { p.text = (p.text || "") + d.delta; p.since = d.length }
                        else { p.text = ""; p.since = 0; return }   // out of step: from the start
                    }
                    if (!d.done && (d.state === "running" || d.state === "queued")) {
                        root.chatPartial = p.structured ? root.chatReplySoFar(p.text || "") : (p.text || "")
                        return
                    }
                    p.deltaDone = true   // finished: the whole job once more, below, as before
                })
                return
            }
            root.aiSessionCall(root.chatSession, "job", [job], function (raw) {
                p.polling = false
                if (root.chatPending !== p || root.chatJob !== job) return
                var r = root.j(raw) || {}
                var st = String(r.state || (r.job && r.job.state) || "")
                var text = String(r.text !== undefined ? r.text : (r.result && r.result.text) || "")
                if (st === "running" || st === "queued" || st === "") { root.chatPartial = p.structured ? root.chatReplySoFar(text) : text; return }
                chatPoll.stop()
                if (st === "error" && p.structured && !p.proseRetry) {
                    // a provider that refused the json_schema format: the same turn once more, in prose
                    // (a failed turn leaves nothing in AI Gateway's history)
                    p.proseRetry = true; root.chatJob = ""; root.chatProse(); root.chatTurn(p.fresh); return
                }
                if (st !== "done") { root.chatUndoSend(root.chatFailNote(st === "cancelled" ? "cancelled" : String(r.code || r.error || "failed"), r)); return }
                var sess = Object.assign({}, root.chatSession); sess.doc = p.doc; sess.reset = false; root.chatSession = sess
                root.chatDocSent = p.doc
                var parsed = p.structured ? root.chatParseJson(text) : null
                if (p.structured && !parsed) root.chatProse()   // answered, but not as the JSON asked for: prose from now on
                if (!parsed) parsed = root.chatParse(text)
                var m2 = root.chatMsgs.slice()
                m2.push({ role: "assistant", raw: parsed.raw || text, text: parsed.text, edits: parsed.edits, model: r.model || sess.model || "",
                          connector: r.connector || sess.connector || "", privacy: r.privacy || sess.privacy || "" })
                root.chatMsgs = m2
                root.chatBusy = false; root.chatPartial = ""; root.chatJob = ""; root.chatPending = null
                root.chatSave()
            })
        }
    }
    // A JSON answer while it streams in is unfinished JSON: show the "reply" text written so
    // far, and how many edits are being written. Edit cards come only from the finished answer.
    function chatReplySoFar(t) {
        t = String(t || "")
        if (!/^\s*(```(json)?\s*)?\{/.test(t)) return t   // not JSON after all: show it as it is
        var m = /"reply"\s*:\s*"/.exec(t)
        var out = ""
        if (m) {
            for (var i = m.index + m[0].length; i < t.length; i++) {
                var c = t.charAt(i)
                if (c === '"') break
                if (c !== "\\") { out += c; continue }
                if (i + 1 >= t.length) break   // an escape cut in half
                var e = t.charAt(++i)
                if (e === "n") out += "\n"
                else if (e === "t") out += "\t"
                else if (e === "r") continue
                else if (e === "u") { if (i + 4 >= t.length) break; out += String.fromCharCode(parseInt(t.substr(i + 1, 4), 16) || 63); i += 4 }
                else out += e   // \" \\ \/
            }
        }
        var ei = t.indexOf('"edits"')
        var n = ei < 0 ? 0 : (t.substring(ei).match(/"find"\s*:/g) || []).length
        return out + (n ? (out ? "\n\n" : "") + "_Writing " + (n === 1 ? "an edit" : n + " edits") + "…_" : "")
    }
    // A call to the app running the chat session (ai-session/1), through the core.
    function aiSessionCall(sess, method, args, cb) {
        if (!sess || !sess.provider) { cb(JSON.stringify({ ok: false, absent: true, error: "no AI chat session" })); return }
        root.core("ai_call", [method, JSON.stringify({ provider: sess.provider, args: args })], cb)
    }
    function chatCloseSession() {
        if (root.chatSession && root.chatSession.id) root.aiSessionCall(root.chatSession, "closeSession", [root.chatSession.id], function () {})
        root.chatSession = null
    }
    // Where an edit's FIND text is in the document's Markdown: exactly, else ignoring
    // trailing spaces on lines. -1 when it is not there (any more).
    function chatFind(md, find) {
        var i = md.indexOf(find)
        if (i >= 0 && find.trim() && md.indexOf(find, i + 1) < 0) return { at: i, len: find.length }
        return root.chatFindLoose(md, find)
    }
    // Where a quote from the AI is in the Markdown, the way models quote: without the
    // formatting marks (** _ ` ~ \), without the "&nbsp;" lines of empty paragraphs, with
    // other dashes and quotes, other spacing, and with "..." for the rest of a line. Only a
    // quote that fits exactly one place counts. { at, len } in md, at -1 when none (or
    // `ambiguous` when several).
    function chatFindLoose(md, find) {
        var fold = function (t, withMap) {
            var out = [], map = []
            var push = function (c, k) { out.push(c); if (withMap) map.push(k) }
            for (var k = 0; k < t.length; k++) {
                var c = t.charAt(k)
                if (c === "&" && t.substr(k, 6) === "&nbsp;") { k += 5; continue }
                if ("*_`~\\\\​".indexOf(c) >= 0) continue
                if ("—–‒−".indexOf(c) >= 0) c = "-"
                else if ("“”„«»".indexOf(c) >= 0) c = '"'
                else if ("‘’".indexOf(c) >= 0) c = "'"
                else if (c === "…") { push(".", k); push(".", k); c = "." }
                else if (c === " " || c === "\t" || c === "\r") c = " "
                push(c, k)
            }
            // spaces: one between words, none around line ends; blank lines go
            var o2 = [], m2 = []
            for (var j = 0; j < out.length; j++) {
                var ch = out[j]
                if (ch === " ") {
                    if (!o2.length || o2[o2.length - 1] === " " || o2[o2.length - 1] === "\n") continue
                } else if (ch === "\n") {
                    while (o2.length && o2[o2.length - 1] === " ") { o2.pop(); m2.pop() }
                    if (!o2.length || o2[o2.length - 1] === "\n") continue
                }
                o2.push(ch); if (withMap) m2.push(map[j])
            }
            while (o2.length && (o2[o2.length - 1] === " " || o2[o2.length - 1] === "\n")) { o2.pop(); m2.pop() }
            return { text: o2.join(""), map: m2 }
        }
        var doc = fold(md, true), q = fold(find, false).text
        if (!q) return { at: -1, len: 0 }
        var esc = function (t) { return t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") }
        // "..." stands for the rest of its line (or the text up to what follows it on the line)
        var pattern = q.split("\n").map(function (line) {
            return line.split("...").map(esc).join("[^\\n]*?") + (/\.\.\.\s*$/.test(line) ? "[^\\n]*" : "")
        }).join("\n")
        var re = new RegExp(pattern, "g"), hits = [], m
        while ((m = re.exec(doc.text)) !== null) { hits.push(m); if (!m[0].length) re.lastIndex++ ; if (hits.length > 1) break }
        if (hits.length !== 1 || !hits[0][0].length) return { at: -1, len: 0, ambiguous: hits.length > 1 }
        var h = hits[0], at = doc.map[h.index], end = doc.map[h.index + h[0].length - 1] + 1
        // take the formatting marks just outside the quote with it (the ** around a bold word)
        while (at > 0 && "*_`~".indexOf(md.charAt(at - 1)) >= 0) at--
        while (end < md.length && "*_`~".indexOf(md.charAt(end)) >= 0) end++
        return { at: at, len: end - at }
    }
    // Apply edit `ei` of message `mi`: one undo step, saved and (when shared) sent like typing.
    function chatApply(mi, ei) {
        var m = root.chatMsgs[mi]; if (!m || !m.edits || !m.edits[ei]) return false
        var e = m.edits[ei]
        var md = root.editorMarkdown()
        var hit = root.chatFind(md, e.find)
        // the title line the AI saw is the draft's title, not part of its text
        if (hit.at < 0) { root.chatSetEdit(mi, ei, hit.ambiguous ? "ambiguous" : "missing"); return false }
        var next = md.substring(0, hit.at) + e.replace + md.substring(hit.at + hit.len)
        root.commitStep()
        if (root.visualEdit) {
            var htmlBefore = richArea.text, caret = richArea.cursorPosition
            root.loadEditor(next)
            richArea.cursorPosition = Math.min(caret, richArea.length)
            // loading set a new history base; make the change one undo step from the old one
            root.histBase = htmlBefore; root.stepOpen = true; root.stepCaret = caret
            root.commitStep()
            root.editorChanged()
            root.commitStep()
        } else {
            mdArea.remove(hit.at, hit.at + hit.len)
            mdArea.insert(hit.at, e.replace)
            root.editorChanged()
            root.commitStep()
        }
        root.chatSetEdit(mi, ei, "applied")
        return true
    }
    function chatApplyAll(mi) {
        var m = root.chatMsgs[mi], n = 0
        for (var i = 0; m && m.edits && i < m.edits.length; i++) if (m.edits[i].state === "new" && root.chatApply(mi, i)) n++
        root.say(n ? "Applied " + n + " edit" + (n > 1 ? "s" : "") + ". Undo (Ctrl+Z) takes them out one by one." : "Nothing applied")
    }
    function chatSetEdit(mi, ei, state) {
        var msgs = root.chatMsgs.slice(), m = Object.assign({}, msgs[mi]), edits = m.edits.slice()
        edits[ei] = Object.assign({}, edits[ei], { state: state })
        m.edits = edits; msgs[mi] = m
        root.chatMsgs = msgs
        root.chatSave()
    }
    // A draft's title is kept apart from its text; a downloaded file starts with it.
    function withTitle(title, md) {
        var t = String(title || "").trim(), first = String(md || "").replace(/^\s+/, "").split("\n")[0]
        if (!t || /^#\s/.test(first)) return md
        return "# " + t + "\n\n" + md
    }
    // The editor's Markdown as it is now (the draft file can be a moment behind the typing).
    function editorMarkdown() {
        if (!root.draft) return ""
        if (!root.visualEdit) return mdArea.text
        var r = root.coreSync("rich_md", [root.draft.id, richArea.text])
        return (r && r.markdown !== undefined) ? r.markdown : root.draft.markdown
    }
    FileDialog {
        id: backupDialog
        title: "Choose a dDocs/dSheets backup-key file"
        nameFilters: ["Backup keys (*.json)", "All files (*)"]
        onAccepted: root.act("import_backup", [String(selectedFile)], "Backup key imported", function (r) { if (r.portals.length) root.showPortal(r.portals[0].portal) })
    }

    // ── building blocks ──
    component Field: TextField {
        color: Theme.palette.text
        placeholderTextColor: Theme.palette.textPlaceholder
        font.pixelSize: Theme.typography.primaryText
        font.family: Theme.typography.publicSans
        leftPadding: 12
        rightPadding: 12
        implicitHeight: 40
        selectByMouse: true
        background: Rectangle {
            color: Theme.palette.backgroundInset
            radius: Theme.spacing.radiusLarge
            border.color: parent.activeFocus ? Theme.palette.primary : Theme.palette.borderSubtle
        }
    }
    component Hint: LogosText {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        textFormat: Text.PlainText
        font.pixelSize: Theme.typography.secondaryText
        color: Theme.palette.textTertiary
    }
    component Caption: LogosText {
        textFormat: Text.PlainText
        font.pixelSize: 11
        font.weight: Theme.typography.weightMedium
        font.letterSpacing: 0.8
        color: Theme.palette.textTertiary
    }
    // A rounded tile with a glyph: what kind of file a row is.
    component KindTile: Rectangle {
        id: kt
        property string kind: "ddoc"
        property string layout: ""
        property int size: 32
        readonly property color tint: kind === "dsheet" ? Theme.palette.success : (layout === "logos" ? Theme.palette.info : Theme.palette.primary)
        implicitWidth: size
        implicitHeight: size
        radius: Theme.spacing.radiusLarge
        color: Theme.colors.getColor(tint, 0.16)
        LogosIcon {
            anchors.centerIn: parent
            width: Math.round(kt.size * 0.55); height: width
            source: kt.kind === "dsheet" ? LogosIcons.grid : LogosIcons.pages
            color: kt.tint
        }
    }
    // A small status pill (IPFS, Storage, Backed up…).
    component Chip: Rectangle {
        id: chip
        property string text: ""
        property color tint: Theme.palette.textTertiary
        implicitWidth: chipText.implicitWidth + 14
        implicitHeight: 20
        radius: 10
        color: Theme.colors.getColor(tint, 0.14)
        LogosText { id: chipText; anchors.centerIn: parent; textFormat: Text.PlainText; font.pixelSize: 11; font.weight: Theme.typography.weightMedium; color: chip.tint; text: chip.text }
    }
    // A list row: kind tile, title, one line of detail, status chips, optional remove.
    component FileRow: Rectangle {
        id: fr
        property string title: ""
        property string sub: ""
        property string kind: "ddoc"
        property string layout: ""
        property bool active: false
        property bool removable: false
        property bool dim: false
        property bool compact: false
        property var chips: []          // [[text, color], …]
        signal clicked()
        signal removed()
        Layout.fillWidth: true
        implicitHeight: compact ? 44 : 56
        radius: Theme.spacing.radiusLarge
        color: active ? Theme.palette.surfaceRaised : (frMouse.containsMouse ? Theme.palette.hover : "transparent")
        Rectangle { visible: fr.active; width: 3; height: parent.height - 16; radius: 2; anchors.verticalCenter: parent.verticalCenter; color: Theme.palette.primary }
        MouseArea { id: frMouse; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: fr.clicked() }
        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: 10
            anchors.rightMargin: 8
            spacing: 10
            KindTile { kind: fr.kind; layout: fr.layout; size: fr.compact ? 26 : 32; opacity: fr.dim ? 0.5 : 1 }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 1
                LogosText {
                    Layout.fillWidth: true; elide: Text.ElideRight; textFormat: Text.PlainText
                    font.pixelSize: fr.compact ? 13 : Theme.typography.primaryText
                    font.weight: Theme.typography.weightMedium
                    color: fr.dim ? Theme.palette.textTertiary : Theme.palette.text
                    text: fr.title
                }
                LogosText {
                    visible: fr.sub !== ""
                    Layout.fillWidth: true; elide: Text.ElideRight; textFormat: Text.PlainText
                    font.pixelSize: 11; color: Theme.palette.textTertiary; text: fr.sub
                }
            }
            Repeater {
                model: fr.compact ? [] : fr.chips
                Chip { text: modelData[0]; tint: modelData[1] }
            }
            LogosIconButton {
                visible: fr.removable && (frMouse.containsMouse || hovered)
                size: 26; iconSize: 12; flat: true
                iconSource: LogosIcons.close
                iconColor: Theme.palette.textTertiary
                background: Item {}
                onClicked: fr.removed()
            }
        }
    }
    // Sidebar navigation entry.
    component NavItem: Rectangle {
        id: ni
        property string label: ""
        property url icon: ""
        property int count: 0
        property bool active: false
        signal clicked()
        Layout.fillWidth: true
        implicitHeight: 36
        radius: Theme.spacing.radiusLarge
        color: active ? Theme.colors.getColor(Theme.palette.primary, 0.14) : (niMouse.containsMouse ? Theme.palette.hover : "transparent")
        MouseArea { id: niMouse; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: ni.clicked() }
        RowLayout {
            anchors.fill: parent; anchors.leftMargin: 10; anchors.rightMargin: 10; spacing: 10
            LogosIcon { Layout.preferredWidth: 16; Layout.preferredHeight: 16; source: ni.icon; color: ni.active ? Theme.palette.primary : Theme.palette.textSecondary }
            LogosText { Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 13; font.weight: Theme.typography.weightMedium; color: ni.active ? Theme.palette.text : Theme.palette.textSecondary; text: ni.label }
            LogosText { visible: ni.count > 0; textFormat: Text.PlainText; font.pixelSize: 11; color: Theme.palette.textTertiary; text: String(ni.count) }
        }
    }
    // A raised panel.
    component Card: Rectangle {
        color: Theme.palette.backgroundSecondary
        radius: Theme.spacing.radiusXlarge
        border.color: Theme.palette.borderHairline
    }
    // Home-screen action card.
    component ActionCard: Rectangle {
        id: ac
        property string title: ""
        property string text: ""
        property url icon: ""
        property color tint: Theme.palette.primary
        signal clicked()
        Layout.fillWidth: true
        Layout.preferredHeight: 150
        radius: Theme.spacing.radiusXlarge
        color: acMouse.containsMouse ? Theme.palette.surface : Theme.palette.backgroundSecondary
        border.color: acMouse.containsMouse ? Theme.colors.getColor(ac.tint, 0.5) : Theme.palette.borderHairline
        MouseArea { id: acMouse; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: ac.clicked() }
        ColumnLayout {
            anchors.fill: parent; anchors.margins: 18; spacing: 8
            Rectangle {
                Layout.preferredWidth: 36; Layout.preferredHeight: 36; radius: Theme.spacing.radiusLarge
                color: Theme.colors.getColor(ac.tint, 0.16)
                LogosIcon { anchors.centerIn: parent; width: 18; height: 18; source: ac.icon; color: ac.tint }
            }
            LogosText { Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 15; font.weight: Theme.typography.weightBold; text: ac.title }
            Hint { text: ac.text }
            Item { Layout.fillHeight: true }
        }
    }

    readonly property var tabs: root.docv && root.docv.tabs ? root.docv.tabs : []
    readonly property var draftTabs: root.draft && root.draft.preview && root.draft.preview.length !== undefined ? root.draft.preview : []
    readonly property var activeTabs: root.mode === "edit" ? root.draftTabs : root.tabs
    readonly property var sheets: root.mode === "doc" && root.docv && root.docv.sheets ? root.docv.sheets : []
    readonly property var blocks: root.activeTabs.length > 0 ? root.activeTabs[Math.min(root.tabIndex, root.activeTabs.length - 1)].blocks : []
    readonly property var sheet: root.sheets.length > 0 ? root.sheets[Math.min(root.sheetIndex, root.sheets.length - 1)] : null

    function shortId(s) { s = String(s || ""); return s.length > 14 ? s.slice(0, 6) + "…" + s.slice(-4) : s }
    function colName(i) { var s = ""; i += 1; while (i > 0) { var m = (i - 1) % 26; s = String.fromCharCode(65 + m) + s; i = Math.floor((i - 1) / 26) } return s }
    function when(ms) { return ms ? new Date(ms).toLocaleString() : "" }
    // "just now", "5 min ago", "3 h ago", "yesterday", "12 Sep", "12 Sep 2025"
    function ago(ms) {
        if (!ms) return ""
        var s = (Date.now() - ms) / 1000
        if (s < 60) return "just now"
        if (s < 3600) return Math.floor(s / 60) + " min ago"
        if (s < 86400) return Math.floor(s / 3600) + " h ago"
        if (s < 2 * 86400) return "yesterday"
        if (s < 7 * 86400) return Math.floor(s / 86400) + " days ago"
        var d = new Date(ms), now = new Date()
        var mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][d.getMonth()]
        return d.getDate() + " " + mon + (d.getFullYear() !== now.getFullYear() ? " " + d.getFullYear() : "")
    }
    function backupChips(bk) {
        if (!bk) return []
        var c = []
        if (bk.ipfs) c.push(["IPFS", Theme.palette.info])
        if (bk.storage) c.push(["Storage", Theme.palette.success])
        if (!bk.ipfs && !bk.storage) c.push(["On this device", Theme.palette.textSecondary])
        return c
    }
    property string portalFilter: ""
    property string filesKind: "all"            // My files: all, fileverse, local, shared
    readonly property var shownPortalFiles: {
        var q = root.portalFilter.trim().toLowerCase(), k = root.filesKind
        var fv = (k === "all" || k === "fileverse") ? root.portalFiles.map(function (f) { return Object.assign({ src: "fileverse" }, f) }) : []
        var loc = (k === "fileverse") ? [] : (root.snap.drafts || []).filter(function (d) { return k !== "shared" || d.shared })
                                              .map(function (d) { return Object.assign({ src: "local" }, d) })
        var all = fv.concat(loc).filter(function (f) {
            return q === "" || String(f.title || "").toLowerCase().indexOf(q) >= 0 || (f.src === "fileverse" && String(f.fileId) === q)
        })
        all.sort(function (a, b) { return (b.updatedAt || 0) - (a.updatedAt || 0) })
        return all
    }
    function fileChips(f) {
        if (f.src === "local") {
            var c = [["Local", Theme.palette.textSecondary]]
            if (f.shared) c.push(["Shared", Theme.palette.success])
            if (f.link) c.push(["Published", Theme.palette.info])
            return c
        }
        return [["Fileverse", Theme.palette.primary]].concat(root.backupChips(root.portalBackup[String(f.fileId)]))
    }

    Rectangle { anchors.fill: parent; color: Theme.palette.background }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // ── sidebar ──
        Rectangle {
            Layout.preferredWidth: 280
            Layout.fillHeight: true
            color: Theme.palette.backgroundSecondary
            Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: Theme.palette.borderHairline }

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.spacing.large
                spacing: Theme.spacing.small

                // brand
                RowLayout {
                    Layout.fillWidth: true
                    Layout.bottomMargin: Theme.spacing.small
                    spacing: 10
                    Rectangle {
                        Layout.preferredWidth: 34; Layout.preferredHeight: 34; radius: Theme.spacing.radiusLarge
                        gradient: Gradient {
                            GradientStop { position: 0; color: Theme.palette.accentOrange }
                            GradientStop { position: 1; color: Theme.palette.accentOrangeDeep }
                        }
                        LogosText { anchors.centerIn: parent; textFormat: Text.PlainText; font.pixelSize: 17; font.weight: Theme.typography.weightBold; color: Theme.palette.backgroundBlack; text: "F" }
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        LogosText { textFormat: Text.PlainText; font.pixelSize: 16; font.weight: Theme.typography.weightBold; text: "Fileverse" }
                        LogosText { textFormat: Text.PlainText; font.pixelSize: 11; color: Theme.palette.textTertiary; text: "self-sovereign · no Fileverse server" }
                    }
                }

                NavItem { label: "Recent"; icon: LogosIcons.list; count: (root.snap.recent || []).length; active: root.section === "recent"; onClicked: root.section = "recent" }
                NavItem { label: "My files"; icon: LogosIcons.pages; count: root.portalFiles.length + (root.snap.drafts || []).length; active: root.section === "files"; onClicked: { root.section = "files"; root.showFiles() } }
                NavItem { label: "Local"; icon: LogosIcons.copy; count: (root.snap.drafts || []).length; active: root.section === "drafts"; onClicked: root.section = "drafts" }

                Rectangle { Layout.fillWidth: true; Layout.topMargin: 6; Layout.bottomMargin: 6; height: 1; color: Theme.palette.borderHairline }

                // Recent
                ColumnLayout {
                    visible: root.section === "recent"
                    Layout.fillWidth: true; Layout.fillHeight: true
                    spacing: 4
                    Caption { text: "RECENTLY OPENED" }
                    ScrollView {
                        Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                        ColumnLayout {
                            width: 248; spacing: 2
                            Repeater {
                                model: root.snap.recent || []
                                FileRow {
                                    compact: true
                                    kind: modelData.kind || "ddoc"; layout: modelData.layout || ""
                                    title: modelData.title || "Untitled"
                                    sub: (root.draftIdFor(modelData.url) ? "Local" : modelData.layout === "logos" ? "Published here" : (modelData.url.indexOf("fvown://") === 0 ? "My files" : "Shared link"))
                                         + (modelData.openedAt ? " · " + root.ago(modelData.openedAt) : "")
                                         + (modelData.kept && modelData.keptOn === root.node.peerId ? " · IPFS" : "") + (modelData.inStorage ? " · Storage" : "")
                                    active: root.mode === "doc" && root.docUrl === modelData.url
                                    removable: true
                                    onClicked: { var did = root.draftIdFor(modelData.url); if (did) { root.section = "drafts"; root.openDraft(did) } else root.open(modelData.url) }
                                    onRemoved: { var u = modelData.url; root.confirm("Remove “" + (modelData.title || "Untitled") + "” from Recent?",
                                        "Its link, with the key that opens it, is forgotten on this device. To open it again you need the link again (a copy you edited stays in Local).",
                                        "Remove", function () { root.act("forget", [u], "Removed from Recent") }) }
                                }
                            }
                            Hint { visible: (root.snap.recent || []).length === 0; text: "Documents you open appear here." }
                        }
                    }
                }

                // My files (backup keys)
                ColumnLayout {
                    visible: root.section === "files"
                    Layout.fillWidth: true; Layout.fillHeight: true
                    spacing: 4
                    Caption { text: "PORTALS" }
                    ScrollView {
                        Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                        ColumnLayout {
                            width: 248; spacing: 2
                            Repeater {
                                model: root.snap.portals || []
                                FileRow {
                                    compact: true
                                    kind: "portal"
                                    title: "Portal " + root.shortId(modelData.portal)
                                    sub: (modelData.kind === "legacy" ? "docs.fileverse.io (legacy)" : (modelData.kind === "pq" ? "post-quantum only" : (modelData.source || "current")))
                                    active: root.mode === "portal" && root.portal === modelData.portal
                                    removable: true
                                    onClicked: root.showPortal(modelData.portal)
                                    onRemoved: { var pt = modelData.portal; root.confirm("Remove this portal's keys?",
                                        "The backup keys of " + root.shortId(pt) + " are deleted from this device. To list and open its files again you need the backup-key file again. The files themselves are not touched.",
                                        "Remove keys", function () { root.act("forget_portal", [pt], "Keys removed from this device") }) }
                                }
                            }
                            LogosButton { Layout.fillWidth: true; Layout.topMargin: 6; compact: true; text: "Import backup key…"; onClicked: root.mode = "import" }
                        }
                    }
                }

                // Drafts
                ColumnLayout {
                    visible: root.section === "drafts"
                    Layout.fillWidth: true; Layout.fillHeight: true
                    spacing: 4
                    Caption { text: "ON THIS DEVICE" }
                    ScrollView {
                        Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                        ColumnLayout {
                            width: 248; spacing: 2
                            LogosButton { Layout.fillWidth: true; compact: true; variant: LogosButton.Variant.Primary; text: "New document"; onClicked: root.newDraft("Untitled", "# Untitled\n\n", "") }
                            LogosButton { Layout.fillWidth: true; Layout.bottomMargin: 6; compact: true; text: root.joinOpen ? "Cancel" : "Join a shared document…"; onClicked: { root.joinOpen = !root.joinOpen; if (root.joinOpen && !(root.mode === "edit" && root.viaReflection)) root.probeReflection() } }
                            ColumnLayout {
                                visible: root.joinOpen
                                Layout.fillWidth: true; Layout.bottomMargin: 6; spacing: 4
                                Field { id: joinField; Layout.fillWidth: true; placeholderText: "Paste the id someone sent you"; onAccepted: root.joinShared(text) }
                                LogosButton { Layout.fillWidth: true; compact: true; variant: LogosButton.Variant.Primary; text: "Join"; enabled: joinField.text.trim().length >= 64; onClicked: root.joinShared(joinField.text) }
                                Caption { visible: root.reflectionAvailable && root.reflNotes.length > 0; text: "NOTES ON THIS DEVICE"; Layout.topMargin: 6 }
                                Repeater {
                                    model: root.reflectionAvailable ? root.reflNotes : []
                                    FileRow {
                                        compact: true
                                        Layout.fillWidth: true
                                        title: modelData.name || "Untitled note"
                                        sub: "Reflection" + (modelData.online > 0 ? " · " + modelData.online + " online" : "")
                                        onClicked: root.openNote(modelData)
                                    }
                                }
                            }
                            Repeater {
                                model: root.snap.drafts || []
                                FileRow {
                                    compact: true
                                    kind: "ddoc"; layout: "logos"
                                    title: modelData.title || "Untitled"
                                    sub: (modelData.shared ? "Shared · " : "") + (modelData.link ? "Published " + root.ago(modelData.publishedAt) : "Edited " + root.ago(modelData.updatedAt))
                                    active: root.mode === "edit" && root.draft && root.draft.id === modelData.id
                                    removable: true
                                    onClicked: root.openDraft(modelData.id)
                                    onRemoved: { var did = modelData.id, shared = !!modelData.shared, published = !!modelData.link
                                        root.confirm("Delete “" + (modelData.title || "Untitled") + "”?",
                                            "It is deleted from this device, with its AI chat. This cannot be undone."
                                            + (published ? " The published copy stays where it was published." : "")
                                            + (shared ? " The others in the shared document keep their copies." : ""),
                                            "Delete", function () { root.act("draft_delete", [did], "Deleted from this device", function () { if (root.draft && root.draft.id === did) { root.draft = null; root.mode = "home" } }) }) }
                                }
                            }
                        }
                    }
                }

                // status
                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: statusCol.implicitHeight + 20
                    radius: Theme.spacing.radiusLarge
                    color: Theme.palette.backgroundInset
                    ColumnLayout {
                        id: statusCol
                        anchors.left: parent.left; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                        anchors.leftMargin: 12; anchors.rightMargin: 6
                        spacing: 4
                        Repeater {
                            model: [
                                ["Core " + (root.snap.version || ""), root.coreReachable, root.coreReachable ? "" : "not answering"],
                                ["IPFS node", !!root.node.reachable, root.node.reachable ? (root.node.peers !== undefined ? root.node.peers + " peers" : "on") : (root.ipfsMode === "needed" && !/install/.test(root.node.error || "") ? "when needed" : "off")],
                                ["Logos Storage", !!root.storage.available, root.storage.available ? "on" : "off"]
                            ]
                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 8
                                Rectangle { Layout.preferredWidth: 7; Layout.preferredHeight: 7; radius: 4; color: modelData[1] ? Theme.palette.success : Theme.palette.textMuted }
                                LogosText { Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 11; color: Theme.palette.textSecondary; text: modelData[0] }
                                LogosText { textFormat: Text.PlainText; font.pixelSize: 11; color: Theme.palette.textTertiary; text: modelData[2] }
                            }
                        }
                    }
                }
                LogosButton {
                    Layout.fillWidth: true
                    compact: true
                    text: root.showSettings ? "Close settings" : "Settings"
                    onClicked: { root.showSettings = !root.showSettings; gwField.text = (root.snap.gateways || []).join("\n"); root.refreshNode(); root.refreshStorage() }
                }
            }
        }

        // ── main pane ──
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: Theme.spacing.xlarge
            spacing: Theme.spacing.large

            // open bar
            RowLayout {
                Layout.fillWidth: true
                visible: root.mode !== "edit"
                spacing: Theme.spacing.small
                Field {
                    id: linkField
                    Layout.fillWidth: true
                    placeholderText: "Paste a Fileverse share link (ddocs.new, dsheets.new, docs.fileverse.io) or a fvbc:// link"
                    onAccepted: root.open(text)
                }
                LogosButton {
                    compact: true
                    variant: LogosButton.Variant.Primary
                    enabled: !root.busyWith("open")
                    text: root.busyWith("open") ? ("Opening… " + root.elapsed("open") + "s") : "Open"
                    onClicked: root.open(linkField.text)
                }
            }

            LogosNotice {
                visible: root.error !== ""
                Layout.fillWidth: true
                severity: LogosNotice.Error
                message: root.error
                closable: true
                onDismissed: root.error = ""
            }

            // settings (scrolls on its own: it is taller than a small window)
            ScrollView {
                id: settingsScroll
                visible: root.showSettings
                Layout.fillWidth: true
                Layout.preferredHeight: settingsCard.implicitHeight
                Layout.maximumHeight: Math.max(200, root.height - 150)
                contentWidth: availableWidth
                clip: true
                Card {
                    id: settingsCard
                    width: settingsScroll.availableWidth
                    implicitHeight: settingsCol.implicitHeight + 32
                    ColumnLayout {
                        id: settingsCol
                        anchors.fill: parent
                        anchors.margins: 16
                        spacing: Theme.spacing.small
                        RowLayout {
                            Layout.fillWidth: true
                            LogosText { Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 16; font.weight: Theme.typography.weightBold; text: "Settings" }
                            LogosButton { compact: true; text: "Close"; onClicked: root.showSettings = false }
                        }
                        Caption { text: "PAPER"; Layout.topMargin: 4 }
                        Hint { text: "The colours documents are read and written in (the editors, the preview, documents and slides). The rest of the app keeps Basecamp's colours." }
                        RowLayout {
                            Repeater {
                                model: [{ k: "basecamp", t: "Basecamp" }, { k: "light", t: "Light" }, { k: "dark", t: "Dark" }]
                                LogosButton {
                                    compact: true; text: modelData.t
                                    variant: root.paper === modelData.k ? LogosButton.Variant.Primary : LogosButton.Variant.Secondary
                                    onClicked: root.setPaper(modelData.k)
                                }
                            }
                        }
                        Caption { text: "SHARED DOCUMENTS: NETWORKS"; Layout.topMargin: 4 }
                        Hint {
                            text: root.networksSettable
                                  ? "Where shared documents are published (Reflection's engine, reflection_core, publishes them; the same setting as in Reflection). Logos: Logos Delivery. p2panda: the p2panda network of Reflection's other apps. Both: everyone on either can join, and edits are passed between the two."
                                  : "Shared documents are published by Reflection's engine (reflection_core), now on " + root.networksLabel(root.reflSyncMode === "p2panda" ? "p2panda" : "logos") + ". Choosing Logos, p2panda or both here needs a newer reflection_core."
                        }
                        RowLayout {
                            visible: root.networksSettable
                            Repeater {
                                model: [{ k: "logos", t: "Logos" }, { k: "p2panda", t: "p2panda" }, { k: "both", t: "Both" }]
                                LogosButton {
                                    compact: true; text: modelData.t
                                    variant: root.reflSettings && root.reflSettings.networks === modelData.k ? LogosButton.Variant.Primary : LogosButton.Variant.Secondary
                                    onClicked: root.refl("set_settings", [JSON.stringify({ networks: modelData.k })], function (raw) {
                                        var r = root.j(raw)
                                        if (r && r.ok) { if (r.settings) root.reflSettings = r.settings; root.say("Shared documents: " + root.networksLabel(modelData.k)); root.refreshCollab() }
                                        else root.fail((r && r.error) || "reflection_core did not take the setting")
                                    })
                                }
                            }
                        }
                        Caption { text: "AI PROMPTS"; Layout.topMargin: 4 }
                        Hint { text: "What Fileverse asks the AI in Write with AI and AI chat. Change them to your liking; they are kept on this device only and never shared. Instructions of your own for every app can also be set in AI Gateway." }
                        Repeater {
                            model: root.aiPromptList
                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.spacing.small
                                LogosText { Layout.fillWidth: true; elide: Text.ElideRight; textFormat: Text.PlainText; font.pixelSize: 13; text: modelData[1] }
                                LogosText { visible: root.aiPromptEdited(modelData[0], modelData[2]); textFormat: Text.PlainText; font.pixelSize: 11; color: Theme.palette.primary; text: "yours" }
                                LogosButton { compact: true; text: "Edit"; onClicked: root.promptEdit = { key: modelData[0], title: modelData[1], def: modelData[2], text: root.aiPromptNow(modelData[0], modelData[2]) } }
                            }
                        }
                        Caption { text: "MY AI ACTIONS"; Layout.topMargin: 4 }
                        Hint { text: root.aiMine.length ? "Buttons of your own in Write with AI, after Fileverse's." : "Add buttons of your own to Write with AI: a name and what to ask, e.g. \"Make it friendlier\"." }
                        Repeater {
                            model: root.aiMine
                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.spacing.small
                                LogosText { Layout.fillWidth: true; elide: Text.ElideRight; textFormat: Text.PlainText; font.pixelSize: 13; text: modelData.name }
                                LogosButton { compact: true; text: "Edit"; onClicked: root.promptEdit = { mine: index, name: modelData.name, text: modelData.prompt, how: modelData.how || "replace" } }
                                LogosButton { compact: true; text: "Remove"; onClicked: { var i = index, n = modelData.name; root.confirm("Remove " + n + "?", "The button goes from Write with AI, with its prompt.", "Remove", function () { root.aiSetMine(i, "", "", "") }) } }
                            }
                        }
                        LogosButton { compact: true; text: "Add an action"; onClicked: root.promptEdit = { mine: -1, name: "", text: "", how: "replace" } }
                        Caption { text: "IPFS GATEWAYS"; Layout.topMargin: 4 }
                        Hint { text: "Tried after your own node, one per line. Every block is checked against its hash, so a gateway can fail but cannot change content." }
                        ScrollView {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 80
                            TextArea {
                                id: gwField
                                color: Theme.palette.text
                                font.pixelSize: 13
                                font.family: Theme.typography.mono
                                background: Rectangle { color: Theme.palette.backgroundInset; radius: Theme.spacing.radiusLarge; border.color: Theme.palette.borderSubtle }
                            }
                        }
                        RowLayout {
                            LogosButton {
                                compact: true
                                text: "Save gateways"
                                onClicked: {
                                    var list = gwField.text.split("\n").map(function (s) { return s.trim() }).filter(function (s) { return s !== "" })
                                    root.act("set_gateways", [JSON.stringify(list)], "Gateways saved")
                                }
                            }
                            LogosButton { compact: true; text: "Restore defaults"; onClicked: root.act("set_gateways", ["[]"], "Defaults restored", function (r) { gwField.text = r.gateways.join("\n") }) }
                        }
                        Caption { text: "IPFS NODE"; Layout.topMargin: 8 }
                        Hint { text: "A running IPFS node takes part in the IPFS network and uses bandwidth. Documents also open without it (public gateways and Logos Storage). Off: never started; publishing then goes to Logos Storage only. When needed: started for Keep on my IPFS, Back up and Publish, and stopped 10 minutes after. Always on: started whenever Fileverse runs." }
                        RowLayout {
                            Repeater {
                                model: [{ k: "off", t: "Off" }, { k: "needed", t: "When needed" }, { k: "always", t: "Always on" }]
                                LogosButton {
                                    compact: true; text: modelData.t
                                    variant: root.ipfsMode === modelData.k ? LogosButton.Variant.Primary : LogosButton.Variant.Secondary
                                    onClicked: root.setIpfsMode(modelData.k)
                                }
                            }
                        }
                        LogosText {
                            Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 12
                            color: root.node.reachable ? Theme.palette.success : Theme.palette.textTertiary
                            text: root.node.reachable
                                  ? ((root.node.source === "ipfs_module" ? "The IPFS Node module is running" : "Your Kubo") + " · " + root.shortId(root.node.peerId)
                                     + (root.node.peers !== undefined ? " · " + root.node.peers + " peers · " + root.node.pins + " pinned · " + Math.round((root.node.repoSize || 0) / 1048576) + " MB" : "")
                                     + ". Read from first; Keep, Back up and Publish pin there.")
                                  : root.ipfsMode === "off" ? "Off: Fileverse does not start the IPFS node."
                                  : root.ipfsMode === "needed" && !/install/.test(root.node.error || "") ? "Not running now; it starts when Keep, Back up or Publish needs it."
                                  : "No IPFS node. Install the IPFS Node module, or run Kubo and enter its API and gateway below."
                        }
                        RowLayout {
                            Layout.fillWidth: true
                            Field { id: nodeApiField; Layout.fillWidth: true; placeholderText: "Kubo API, e.g. http://127.0.0.1:5001"; text: root.snap.node ? root.snap.node.api : "" }
                            Field { id: nodeGwField; Layout.fillWidth: true; placeholderText: "Kubo gateway, e.g. http://127.0.0.1:8080"; text: root.snap.node ? root.snap.node.gateway : "" }
                            LogosButton { compact: true; text: "Save node"; onClicked: root.act("set_ipfs_node", [nodeApiField.text, nodeGwField.text], "IPFS node saved", function () { root.refreshNode() }) }
                        }
                        Caption { text: "LOGOS STORAGE · GNOSIS"; Layout.topMargin: 8 }
                        LogosText {
                            Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 12
                            color: root.storage.available ? Theme.palette.success : Theme.palette.textTertiary
                            text: root.storage.available ? ("Logos Storage: available (" + (root.snap.storedBlocks || 0) + " blocks copied there). Read from when IPFS has nothing.") : "Logos Storage is not available in this Basecamp."
                        }
                        Hint { text: "Gnosis RPC: " + (root.snap.rpc || "eth_rpc_module") + ". Change the endpoint (or a proxy/Tor) in the Ethereum RPC app." }
                    }
                }
            }

            // opening a document
            Card {
                visible: root.busyWith("open")
                Layout.fillWidth: true
                implicitHeight: 64
                RowLayout {
                    anchors.fill: parent
                    anchors.margins: 16
                    spacing: 14
                    LogosSpinner { Layout.preferredWidth: 26; Layout.preferredHeight: 26; running: parent.visible }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        LogosText { textFormat: Text.PlainText; font.pixelSize: 14; font.weight: Theme.typography.weightMedium; text: "Opening… " + root.elapsed("open") + "s" }
                        Hint { text: root.progressOf("open") || "Reading the file record on Gnosis, then its hash-checked blocks from your node, gateways or Logos Storage." }
                    }
                }
            }

            // home
            ColumnLayout {
                visible: root.mode === "home" && !root.showSettings
                Layout.fillWidth: true
                Layout.topMargin: Theme.spacing.xlarge
                spacing: Theme.spacing.medium
                LogosText { textFormat: Text.PlainText; font.pixelSize: Theme.typography.titleText; font.weight: Theme.typography.weightBold; text: "Your documents, without the server" }
                Hint { font.pixelSize: Theme.typography.primaryText; text: "Read dDocs and dSheets straight from Gnosis and IPFS, keep copies on your own node and in Logos Storage, and write your own. Keys never leave this device." }
                RowLayout {
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.spacing.medium
                    spacing: Theme.spacing.medium
                    ActionCard { title: "Open a share link"; icon: LogosIcons.search; text: "Paste a link above. It is decrypted here; every block is hash-checked."; onClicked: linkField.forceActiveFocus() }
                    ActionCard { title: "Your files"; icon: LogosIcons.pages; tint: Theme.palette.success; text: "Import your dDocs backup key to list, open and back up your whole portal."; onClicked: { root.section = "files"; root.mode = (root.snap.portals || []).length ? root.mode : "import"; if ((root.snap.portals || []).length) root.showPortal(root.snap.portals[0].portal) } }
                    ActionCard { title: "Write a document"; icon: LogosIcons.copy; tint: Theme.palette.info; text: "Markdown with a live preview, presentable as slides, published to IPFS and Logos Storage."; onClicked: { root.section = "drafts"; root.newDraft("Untitled", "# Untitled\n\n", "") } }
                }
            }

            // import a backup key
            ColumnLayout {
                visible: !root.showSettings && (root.mode === "import")
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                LogosText { text: "Your own files"; font.pixelSize: Theme.typography.panelTitleText; font.weight: Theme.typography.weightBold; textFormat: Text.PlainText }
                Hint { font.pixelSize: 13; text: "dDocs and dSheets let you download backup keys (Settings → Backup). With that file, this app lists and opens every file of your portal straight from Gnosis and IPFS, with no share links and no Fileverse service. The keys are stored only on this device (file mode 600). Files locked with Fileverse's post-quantum scheme cannot be opened yet: its library is not public." }
                RowLayout {
                    Layout.topMargin: 6
                    LogosButton { compact: true; variant: LogosButton.Variant.Primary; text: "Choose backup file…"; onClicked: backupDialog.open() }
                    Hint { text: "or paste its contents:" }
                }
                ScrollView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 140
                    TextArea {
                        id: backupText
                        color: Theme.palette.text
                        font.pixelSize: 12
                        font.family: Theme.typography.mono
                        wrapMode: TextEdit.WrapAnywhere
                        placeholderText: "{ \"portalAddress\": \"0x…\", … }"
                        background: Rectangle { color: Theme.palette.backgroundInset; radius: Theme.spacing.radiusLarge; border.color: Theme.palette.borderSubtle }
                    }
                }
                LogosButton {
                    compact: true
                    text: "Import"
                    onClicked: root.act("import_backup", [backupText.text], "Backup key imported", function (r) { backupText.text = ""; root.section = "files"; if (r.portals.length) root.showPortal(r.portals[0].portal) })
                }
            }

            // a portal's files
            ColumnLayout {
                visible: !root.showSettings && (root.mode === "portal")
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Theme.spacing.medium
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacing.medium
                    ColumnLayout {
                        spacing: 4
                        LogosText { text: "My files"; font.pixelSize: Theme.typography.panelTitleText; font.weight: Theme.typography.weightBold; textFormat: Text.PlainText }
                        RowLayout {
                            spacing: 6
                            Chip {
                                visible: root.portal !== ""
                                text: root.shortId(root.portal)
                                tint: Theme.palette.textSecondary
                                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.copy(root.portal, "Portal address") }
                            }
                            Hint {
                                Layout.fillWidth: false
                                text: root.portal === "" ? ((root.snap.drafts || []).length + " local documents")
                                      : root.portalError !== "" && root.portalFiles.length === 0 ? "Could not read this portal."
                                      : (root.portalFiles.length + " from Fileverse · " + (root.snap.drafts || []).length + " local · "
                                         + Object.keys(root.portalBackup).length + " backed up"
                                         + (root.portalListedAt ? " · listed " + root.ago(root.portalListedAt) : ""))
                            }
                        }
                    }
                    Item { Layout.fillWidth: true }
                    LogosButton {
                        visible: root.portal === ""
                        compact: true
                        text: "Import your Fileverse files…"
                        onClicked: root.mode = "import"
                    }
                    LogosButton {
                        visible: root.portal !== ""
                        compact: true
                        enabled: !root.busyWith("list")
                        text: root.busyWith("list") ? ("Refreshing… " + (root.progressOf("list") || root.elapsed("list") + "s")) : "Refresh"
                        onClicked: root.refreshPortal()
                    }
                    LogosButton {
                        visible: root.portal !== ""
                        compact: true
                        variant: LogosButton.Variant.Primary
                        enabled: !root.busyWith("backup") && root.portalFiles.length > 0
                        text: root.busyWith("backup") ? ("Backing up… " + (root.progressOf("backup") || root.elapsed("backup") + "s")) : "Back up everything"
                        onClicked: root.backupPortal()
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    Repeater {
                        model: [{ k: "all", t: "All" }, { k: "fileverse", t: "Fileverse" }, { k: "local", t: "Local" }, { k: "shared", t: "Shared" }]
                        LogosButton {
                            compact: true; text: modelData.t
                            variant: root.filesKind === modelData.k ? LogosButton.Variant.Primary : LogosButton.Variant.Secondary
                            onClicked: root.filesKind = modelData.k
                        }
                    }
                    Field {
                        Layout.fillWidth: true
                        placeholderText: "Filter by title or number"
                        onTextChanged: root.portalFilter = text
                    }
                }
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    ListView {
                        anchors.fill: parent
                        anchors.margins: 6
                        clip: true
                        spacing: 2
                        model: root.shownPortalFiles
                        ScrollBar.vertical: ScrollBar {}
                        delegate: FileRow {
                            width: ListView.view.width
                            readonly property bool local: modelData.src === "local"
                            kind: local ? "ddoc" : (modelData.kind || "ddoc")
                            layout: local ? "logos" : ""
                            dim: !!modelData.error
                            title: modelData.title || (modelData.error ? "Cannot open" : "Untitled")
                            sub: local ? ("Edited " + root.ago(modelData.updatedAt) + (modelData.link ? " · published " + root.ago(modelData.publishedAt) : ""))
                                 : ("#" + modelData.fileId + (modelData.kind === "dsheet" ? " · Sheet" : " · Doc")
                                 + (modelData.updatedAt ? " · " + root.ago(modelData.updatedAt) : "")
                                 + (modelData.titleFromText ? " · title from text" : "")
                                 + (modelData.error ? " · " + modelData.error : ""))
                            chips: root.fileChips(modelData)
                            onClicked: {
                                if (local) { root.section = "drafts"; root.openDraft(modelData.id) }
                                else if (!modelData.error) root.open(modelData.url)
                            }
                        }
                    }
                    Hint {
                        anchors.centerIn: parent
                        width: parent.width - 60
                        horizontalAlignment: Text.AlignHCenter
                        visible: root.shownPortalFiles.length === 0
                        text: root.busyWith("list") ? "Reading the portal on Gnosis…" : (root.portalFilter !== "" || root.filesKind !== "all" ? "No file matches."
                              : root.portal === "" ? "Nothing here yet: write a document (Local), or import your Fileverse files." : "No files listed yet. Press Refresh.")
                    }
                }
                Hint { visible: root.portal !== ""; text: "Back up everything keeps every Fileverse file's encrypted blocks on this device, pins them on your IPFS node and copies them into Logos Storage, so they stay readable without Fileverse." }
            }

            // document header
            RowLayout {
                visible: !root.showSettings && (root.mode === "doc" && root.docv !== null)
                Layout.fillWidth: true
                spacing: Theme.spacing.medium
                KindTile { Layout.alignment: Qt.AlignTop; size: 40; kind: root.docv ? root.docv.kind : "ddoc"; layout: root.docv ? root.docv.layout : "" }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4
                    LogosText { Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: Theme.typography.panelTitleText; font.weight: Theme.typography.weightBold; text: root.docv ? (root.docv.title || "Untitled") : "" }
                    RowLayout {
                        spacing: 6
                        Hint {
                            Layout.fillWidth: false
                            text: !root.docv ? "" : ((root.docv.kind === "dsheet" ? "Sheet" : "Doc") + " · "
                                  + (root.docv.layout === "logos" ? "published from Basecamp" : (root.docv.layout + " portal " + root.shortId(root.docv.portal) + " #" + root.docv.fileId))
                                  + (root.docv.updatedAt ? " · updated " + root.ago(root.docv.updatedAt) : ""))
                        }
                        Chip { visible: root.kept; text: "On my IPFS"; tint: Theme.palette.info }
                        Chip { visible: root.inStorage; text: "In Logos Storage"; tint: Theme.palette.success }
                        Chip { visible: !!(root.docv && root.docv.titleFromText); text: "title from text"; tint: Theme.palette.textSecondary }
                    }
                }
            }
            Flow {
                visible: !root.showSettings && (root.mode === "doc" && root.docv !== null)
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                LogosButton { visible: root.coreSlides.length > 0; compact: true; variant: LogosButton.Variant.Primary; text: "Present"; onClicked: root.present() }
                LogosButton { visible: root.docDraftId !== ""; compact: true; variant: LogosButton.Variant.Primary; text: "Edit"; onClicked: { root.section = "drafts"; root.openDraft(root.docDraftId) } }
                LogosButton { visible: root.docDraftId === "" && root.docCopyId !== ""; compact: true; text: "Edit my copy"; onClicked: { root.section = "drafts"; root.openDraft(root.docCopyId) } }
                LogosButton { visible: root.docv && root.docv.markdown !== undefined && root.docDraftId === ""; compact: true; text: root.docCopyId !== "" ? "New copy" : "Edit a copy"; onClicked: root.newDraft(root.docv.title || "Untitled", root.docv.markdownBody !== undefined ? root.docv.markdownBody : root.docv.markdown, root.docUrl) }
                LogosButton {
                    visible: root.ipfsUsable && !root.kept
                    compact: true
                    enabled: root.ipfsUsable && !root.busyWith("keep")
                    text: root.busyWith("keep") ? "Pinning…" : "Keep on my IPFS"
                    onClicked: root.keepCopy()
                }
                LogosButton {
                    visible: (root.storage.available || root.inStorage) && !root.inStorage
                    compact: true
                    enabled: root.storage.available && !root.busyWith("storage")
                    text: root.busyWith("storage") ? ("Copying… " + root.elapsed("storage") + "s") : "Copy to Logos Storage"
                    onClicked: root.copyToStorage()
                }
                LogosButton { visible: root.docv && root.docv.markdown !== undefined; compact: true; text: "Copy Markdown"; onClicked: root.copy(root.docv.markdown, "Markdown") }
                LogosButton { visible: root.docv && root.docv.markdown !== undefined; compact: true; text: "Download Markdown"; onClicked: root.downloadMarkdown(root.docv.title, root.docv.markdown) }
                LogosButton { visible: root.docUrl.indexOf("fvbc://") === 0; compact: true; text: "Copy link"; onClicked: root.copy(root.docUrl, "Link") }
                LogosButton { compact: true; text: root.showDetails ? "Hide sources" : "Sources"; onClicked: root.showDetails = !root.showDetails }
            }

            // provenance
            Card {
                visible: !root.showSettings && (root.mode === "doc" && root.showDetails && root.docv !== null)
                Layout.fillWidth: true
                implicitHeight: provCol.implicitHeight + 24
                ColumnLayout {
                    id: provCol
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 2
                    Caption { text: "WHERE THIS CAME FROM" }
                    Repeater {
                        model: root.docv ? root.docv.provenance.steps : []
                        LogosText { Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 12; color: Theme.palette.textSecondary; text: "• " + modelData }
                    }
                    Repeater {
                        model: root.docv ? root.docv.provenance.blobs : []
                        LogosText {
                            Layout.fillWidth: true
                            elide: Text.ElideMiddle
                            textFormat: Text.PlainText
                            font.pixelSize: 12
                            font.family: Theme.typography.mono
                            color: modelData.verified ? Theme.palette.textSecondary : Theme.palette.warning
                            text: (modelData.verified ? "✓ " : "⚠ unverified ") + modelData.cid + " · " + modelData.bytes + " B · " + modelData.gateway
                        }
                    }
                }
            }

            // editor, with the AI chat beside it
            RowLayout {
                visible: !root.showSettings && (root.mode === "edit" && root.draft !== null)
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Theme.spacing.medium
                // editor
                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: Theme.spacing.small
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.spacing.small
                        Field {
                            id: titleField
                            Layout.fillWidth: true
                            font.pixelSize: 22
                            font.weight: Theme.typography.weightBold
                            placeholderText: "Title"
                            background: Item {}
                            onTextEdited: { root.draftDirty = true; previewTimer.restart() }
                            // a title set by opening a document shows from its start when the field is narrow
                            onTextChanged: if (!activeFocus) cursorPosition = 0
                            // Tab or Enter in the title goes on into the text (not to the buttons)
                            Keys.onTabPressed: function (e) { root.focusEditor(); e.accepted = true }
                            Keys.onReturnPressed: function (e) { root.focusEditor(); e.accepted = true }
                            Keys.onEnterPressed: function (e) { root.focusEditor(); e.accepted = true }
                        }
                        Chip {
                            visible: !root.isShared
                            text: root.draftDirty ? "editing…" : "saved on this device"; tint: root.draftDirty ? Theme.palette.warning : Theme.palette.success
                        }
                        LogosButton {
                            // reflection_core sends this device's cursor only when the user says so
                            // (one setting for every note, also Reflection's "Share my cursor")
                            readonly property bool on: !!(root.collab.me && root.collab.me.shareCursor)
                            visible: root.isShared && root.viaReflection; compact: true
                            text: on ? "Hide my cursor" : "Share my cursor"
                            onClicked: root.refl("set_settings", [JSON.stringify({ shareCursor: !on })], function () { root.refreshCollab(); if (!on) root.sendCursor() })
                            HoverHandler { id: cursorHover }
                            ToolTip.visible: cursorHover.hovered
                            ToolTip.text: on ? "The others on Reflection notes see where you are in the text. Same setting as Reflection's \"Share my cursor\"."
                                             : "Show the others on Reflection notes where you are in the text. Same setting as Reflection's \"Share my cursor\"."
                        }
                        LogosButton {
                            visible: root.isShared; compact: true
                            text: "Stop sharing"
                            onClicked: root.stopSharing()
                            HoverHandler { id: leaveHover }
                            ToolTip.visible: leaveHover.hovered
                            ToolTip.text: "Stop syncing this document on this device. Its text stays here, private; the others keep their copies and can go on editing together (the note stays in Reflection)."
                        }
                        LogosButton {
                            visible: !root.isShared && !!root.draft && !root.viaReflection
                            compact: true; text: "Share…"
                            onClicked: root.shareClicked()
                            HoverHandler { id: shareBtnHover }
                            ToolTip.visible: shareBtnHover.hovered
                            ToolTip.text: "Edit this document together with others, live. You get an id to send them; anyone with it can read and write. It also opens in the Reflection apps."
                        }
                        LogosButton { visible: root.coreSlides.length > 0; compact: true; text: "Present"; onClicked: root.present() }
                        LogosButton {
                            compact: true
                            variant: LogosButton.Variant.Primary
                            enabled: !root.busyWith("publish")
                            text: root.busyWith("publish") ? ("Publishing… " + root.elapsed("publish") + "s") : (root.draft && root.draft.published ? "Publish again" : "Publish")
                            onClicked: root.publishDraft()
                        }
                    }
                    // changed on disk (an agent?) while you have unsaved edits here
                    Rectangle {
                        Layout.fillWidth: true
                        visible: !!root.outsideDraft
                        implicitHeight: outsideRow.implicitHeight + 16
                        radius: Theme.spacing.radiusSmall
                        color: Theme.colors.getColor(Theme.palette.primary, 0.16)
                        border.color: Theme.palette.primary
                        RowLayout {
                            id: outsideRow
                            anchors.fill: parent
                            anchors.margins: 8
                            spacing: Theme.spacing.small
                            LogosText {
                                Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 13
                                text: "This document was changed outside the editor (by an agent or another window) while you were typing. Saving is paused."
                            }
                            LogosButton { compact: true; variant: LogosButton.Variant.Primary; text: "Load their version"; onClicked: root.reloadDraft(root.outsideDraft) }
                            LogosButton { compact: true; text: "Keep mine"; onClicked: root.keepMine() }
                        }
                    }
                    Flow {
                        Layout.fillWidth: true
                        visible: !!root.draft
                        spacing: Theme.spacing.small
                        LogosButton { visible: root.isShared; compact: true; text: "Copy id"; onClicked: root.copy(root.collab.id, "Shared document id") }
                        LogosButton { compact: true; text: "Download Markdown"; onClicked: root.downloadMarkdown(titleField.text, root.withTitle(titleField.text, root.editorMarkdown())) }
                        LogosButton {
                            compact: true
                            variant: root.chatOpen ? LogosButton.Variant.Primary : LogosButton.Variant.Secondary
                            text: root.chatBusy ? "💬 AI chat…" : "💬 AI chat"
                            onClicked: root.chatToggle()
                            HoverHandler { id: chatHover }
                            ToolTip.visible: chatHover.hovered
                            ToolTip.text: "Talk with an AI about this document; it can propose edits for you to apply. Uses the AI Gateway app. The chat stays on this device."
                        }
                        LogosButton {
                            compact: true
                            variant: root.aiOpen ? LogosButton.Variant.Primary : LogosButton.Variant.Secondary
                            text: root.aiBusy ? "✨ AI is writing…" : "✨ Write with AI"
                            onClicked: { root.aiOpen = !root.aiOpen; root.aiField = ""; root.aiNote = "" }
                            HoverHandler { id: aiHover }
                            ToolTip.visible: aiHover.hovered
                            ToolTip.text: "Continue, rewrite, summarize, translate or ask about the selection or this paragraph. Uses the AI Gateway app, which shows what is sent and lets you pick the model."
                        }
                        // shared: who else has it open (a dot each), and the transport
                        Row {
                            visible: root.isShared
                            spacing: -6
                            Repeater {
                                model: root.collabOnline
                                Rectangle {
                                    width: 24; height: 24; radius: 12; color: modelData.color; border.color: Theme.palette.background; border.width: 2
                                    LogosText { anchors.centerIn: parent; textFormat: Text.PlainText; font.pixelSize: 11; font.weight: Theme.typography.weightBold; color: "white"; text: (modelData.name || "?").charAt(0).toUpperCase() }
                                    HoverHandler { id: peerHover }
                                    ToolTip.visible: peerHover.hovered; ToolTip.text: modelData.name
                                }
                            }
                        }
                        Chip {
                            visible: root.isShared
                            text: "shared · " + root.networksLabel(root.collab.networks)
                                  + (root.collabOnline.length === 0 ? " · only you here" : (" · " + (root.collabOnline.length + 1) + " here"))
                            tint: root.collab.transport && root.collab.transport.connected ? Theme.palette.success : Theme.palette.warning
                            HoverHandler { id: shareHover }
                            ToolTip.visible: shareHover.hovered
                            ToolTip.text: "A Reflection note: also opens in the Reflection apps. Published by reflection_core on " + root.networksLabel(root.collab.networks) + " (Settings in Fileverse or Reflection)."
                                          + (root.networkStatusText(root.collab.networkStatus) !== "" ? " " + root.networkStatusText(root.collab.networkStatus)
                                             : (root.collab.transport && root.collab.transport.status ? " " + root.collab.transport.status : ""))
                        }
                        // reflection_core 0.6.2+: changes still on their way, either direction
                        Chip {
                            readonly property var d: root.collab.delivery
                            // loroPending (0.6.3+): received changes wait for one this device never got
                            readonly property bool behind: !!d && (!!d.catchingUp || !!d.loroPending)
                            visible: root.isShared && !!d && (behind || !!d.unpublished)
                            text: behind ? "catching up…" : "sending…"
                            tint: Theme.palette.warning
                            HoverHandler { id: deliveryHover }
                            ToolTip.visible: deliveryHover.hovered
                            ToolTip.text: behind
                                          ? "Some of the others' changes have not arrived yet. reflection_core keeps asking for them; the text fills in when they come. Keep the document open."
                                          : "Your latest changes have not been sent yet. They go out as soon as the network lets them."
                        }
                        Chip { visible: root.viaReflection && !root.isShared; text: "shared · reflection_core not answering"; tint: Theme.palette.warning }
                        LogosButton { visible: !!(root.draft && root.draft.published); compact: true; text: "Copy link"; onClicked: root.copy(root.draft.published.link, "Link") }
                        LogosButton { visible: !!(root.draft && root.draft.published); compact: true; text: "Open published"; onClicked: root.open(root.draft.published.link) }
                        LogosButton { visible: !!(root.draft && root.draft.sourceUrl); compact: true; text: "Open original"; onClicked: root.open(root.draft.sourceUrl) }
                        Hint {
                            text: root.draft && root.draft.published
                                  ? ("Published " + root.ago(root.draft.published.at) + ". Each publish makes a new link.")
                                  : ""
                        }
                    }

                    // toolbar
                    Rectangle {
                        Layout.fillWidth: true
                        // wraps onto a second line when the AI chat leaves it too little room
                        implicitHeight: tbFlow.implicitHeight + 12
                        radius: Theme.spacing.radiusLarge
                        color: Theme.palette.backgroundSecondary
                        border.color: Theme.palette.borderHairline
                        Flow {
                            id: tbFlow
                            anchors { left: parent.left; right: parent.right; top: parent.top; leftMargin: 6; rightMargin: 6; topMargin: 6 }
                            spacing: 2
                            Repeater {
                                model: root.visualEdit ? [
                                    ["Text", "p", "Normal text", function () { root.setBlock("p") }],
                                    ["H1", "h1", "Heading 1 (type # and a space)", function () { root.toggleBlock("h1") }],
                                    ["H2", "h2", "Heading 2 (##)", function () { root.toggleBlock("h2") }],
                                    ["H3", "h3", "Heading 3 (###)", function () { root.toggleBlock("h3") }],
                                    ["|", "", "", null],
                                    ["B", "", "Bold (Ctrl+B)", function () { root.toggleFont("bold") }],
                                    ["I", "", "Italic (Ctrl+I)", function () { root.toggleFont("italic") }],
                                    ["S", "", "Strikethrough", function () { root.toggleFont("strikeout") }],
                                    ["</>", "", "Inline code", function () { root.wrapSelection(function (t) { return root.codeSpan(t) + "\u200b" }) }],
                                    ["Link", "", "Link (Ctrl+K)", function () { root.askLink() }],
                                    ["|", "", "", null],
                                    ["• List", "ul", "Bulleted list (-)", function () { root.toggleBlock("ul") }],
                                    ["1. List", "ol", "Numbered list (1.)", function () { root.toggleBlock("ol") }],
                                    ["☐ Task", "task", "Checklist ([] and a space); click again to tick", function () { root.toggleTask() }],
                                    ["❝ Quote", "quote", "Quote (>)", function () { root.toggleBlock("quote") }],
                                    ["{ } Code", "code", "Code block", function () { root.toggleBlock("code") }],
                                    ["|", "", "", null],
                                    ["🖼 Image", "", "Insert a picture (kept with the document; encrypted when published)", function () { imageDialog.open() }],
                                    ["▥ Columns", "", "Two columns side by side, e.g. an image beside text", function () { root.insertColumns() }],
                                    ["|", "", "", null],
                                    ["↶", "", "Undo (Ctrl+Z)", function () { root.undoStep() }],
                                    ["↷", "", "Redo (Ctrl+Shift+Z or Ctrl+Y)", function () { root.redoStep() }]
                                ] : []
                                Item {
                                    implicitWidth: modelData[0] === "|" ? 9 : tbLabel.implicitWidth + 16
                                    implicitHeight: 30
                                    Rectangle { visible: modelData[0] === "|"; anchors.centerIn: parent; width: 1; height: 18; color: Theme.palette.borderHairline }
                                    Rectangle {
                                        visible: modelData[0] !== "|"
                                        anchors.fill: parent
                                        radius: Theme.spacing.radiusMedium
                                        readonly property bool on: modelData[1] !== "" && (root.blockKind === modelData[1] || (modelData[1] === "task" && root.blockKind === "done"))
                                        color: on ? Theme.colors.getColor(Theme.palette.primary, 0.18) : (tbMouse.containsMouse ? Theme.palette.hover : "transparent")
                                        LogosText {
                                            id: tbLabel
                                            anchors.centerIn: parent
                                            textFormat: Text.PlainText
                                            font.pixelSize: 12
                                            font.weight: Theme.typography.weightMedium
                                            font.bold: modelData[0] === "B"
                                            font.italic: modelData[0] === "I"
                                            font.strikeout: modelData[0] === "S"
                                            color: parent.on ? Theme.palette.primary : Theme.palette.textSecondary
                                            text: modelData[1] === "task" && root.blockKind === "done" ? "☑ Task" : modelData[0]
                                        }
                                        MouseArea { id: tbMouse; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: modelData[3]() }
                                        LogosToolTip { text: modelData[2]; visible: tbMouse.containsMouse && text !== "" }
                                    }
                                }
                            }
                            Hint { visible: !root.visualEdit; width: implicitWidth + 12; height: 30; verticalAlignment: Text.AlignVCenter; leftPadding: 6; text: "Markdown source. Switch back to Visual to edit formatted text." }
                            // Visual | Markdown
                            Rectangle {
                                implicitWidth: modeRow.implicitWidth + 4
                                implicitHeight: 30
                                radius: 15
                                color: Theme.palette.backgroundInset
                                Row {
                                    id: modeRow
                                    anchors.centerIn: parent
                                    Repeater {
                                        model: [["Visual", true], ["Markdown", false]]
                                        Rectangle {
                                            width: modeLabel.implicitWidth + 20; height: 26; radius: 13
                                            color: root.visualEdit === modelData[1] ? Theme.palette.surfaceRaised : "transparent"
                                            LogosText { id: modeLabel; anchors.centerIn: parent; textFormat: Text.PlainText; font.pixelSize: 12; color: root.visualEdit === modelData[1] ? Theme.palette.text : Theme.palette.textTertiary; text: modelData[0] }
                                            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.setVisualEdit(modelData[1]) }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // link prompt
                    RowLayout {
                        Layout.fillWidth: true
                        visible: root.linkRange !== null
                        spacing: Theme.spacing.small
                        Field {
                            id: linkUrl
                            Layout.fillWidth: true
                            placeholderText: "https://…"
                            onAccepted: root.applyLink()
                            Keys.onEscapePressed: { root.linkRange = null; richArea.forceActiveFocus() }
                        }
                        LogosButton { compact: true; variant: LogosButton.Variant.Primary; text: "Add link"; onClicked: root.applyLink() }
                        LogosButton { compact: true; text: "Cancel"; onClicked: { root.linkRange = null; richArea.forceActiveFocus() } }
                    }

                    // Write with AI: the actions, then the answer to review
                    Flow {
                        Layout.fillWidth: true
                        visible: root.aiOpen
                        spacing: Theme.spacing.small
                        Repeater {
                            model: root.aiActionsNow
                            LogosButton { compact: true; enabled: !root.aiBusy; text: modelData[0]; onClicked: root.aiRun(modelData[0], modelData[1], modelData[2], modelData[3]) }
                        }
                        LogosButton { compact: true; enabled: !root.aiBusy; text: "Translate…"; onClicked: { root.aiField = "translate"; aiInput.text = ""; aiInput.forceActiveFocus() } }
                        LogosButton { compact: true; enabled: !root.aiBusy; text: "Ask AI…"; onClicked: { root.aiField = "ask"; aiInput.text = ""; aiInput.forceActiveFocus() } }
                        Hint { visible: !root.aiBusy && root.aiField === "" && root.aiNote === ""; text: "Works on the selection, or the paragraph the cursor is in (Summarize and Ask AI read the whole document when nothing is selected). You see the answer before anything changes." }
                        Hint { visible: !root.aiBusy && root.aiNote !== ""; color: Theme.palette.warning; text: root.aiNote }
                        Hint { visible: root.aiBusy; text: "Waiting for the AI… (AI Gateway shows what is sent and asks you first)" }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        visible: root.aiOpen && root.aiField !== ""
                        spacing: Theme.spacing.small
                        Field {
                            id: aiInput
                            Layout.fillWidth: true
                            placeholderText: root.aiField === "translate" ? "Into which language? e.g. Czech" : "What should the AI do? e.g. Turn this into a bulleted list"
                            onAccepted: aiGo.clicked()
                            Keys.onEscapePressed: root.aiField = ""
                        }
                        LogosButton {
                            id: aiGo
                            compact: true; variant: LogosButton.Variant.Primary; enabled: !root.aiBusy && aiInput.text.trim() !== ""
                            text: root.aiField === "translate" ? "Translate" : "Ask"
                            onClicked: {
                                var q = aiInput.text.trim()
                                if (!q) return
                                if (root.aiField === "translate") root.aiRun("Translate", root.aiTranslateNow.indexOf("{language}") >= 0 ? root.aiTranslateNow.split("{language}").join(q) : root.aiTranslateNow + "\n\nLanguage: " + q, "target", "replace")
                                else root.aiRun("Ask AI", q, "targetOrAll", "below")
                                root.aiField = ""
                            }
                        }
                        LogosButton { compact: true; text: "Cancel"; onClicked: root.aiField = "" }
                    }
                    Rectangle {
                        Layout.fillWidth: true
                        visible: root.aiResult !== null
                        implicitHeight: aiCol.implicitHeight + 20
                        radius: Theme.spacing.radiusLarge
                        color: Theme.colors.getColor(Theme.palette.primary, 0.08)
                        border.color: Theme.colors.getColor(Theme.palette.primary, 0.5)
                        ColumnLayout {
                            id: aiCol
                            anchors { left: parent.left; right: parent.right; top: parent.top; margins: 10 }
                            spacing: 6
                            LogosText {
                                Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 12; color: Theme.palette.textSecondary; elide: Text.ElideRight
                                text: root.aiResult ? ("✨ " + root.aiResult.label
                                      + (root.aiResult.model ? " · " + root.aiResult.model : "")
                                      + (root.aiResult.privacy ? " · " + root.aiResult.privacy : "")
                                      + " — review it, then choose where it goes") : ""
                            }
                            ScrollView {
                                Layout.fillWidth: true
                                Layout.preferredHeight: Math.min(aiText.implicitHeight + 8, 220)
                                clip: true
                                TextArea {
                                    id: aiText
                                    readOnly: true; selectByMouse: true; wrapMode: TextEdit.Wrap
                                    textFormat: TextEdit.MarkdownText; font.pixelSize: 14; font.family: Theme.typography.publicSans
                                    color: Theme.palette.text; background: Item {}
                                    text: root.aiResult ? root.aiResult.text : ""
                                }
                            }
                            Flow {
                                Layout.fillWidth: true
                                spacing: Theme.spacing.small
                                readonly property bool intact: root.aiRangeIntact(root.aiResult) && root.aiResult.a !== root.aiResult.b
                                LogosButton { visible: !!root.aiResult && root.aiResult.how === "replace"; enabled: parent.intact; compact: true; variant: LogosButton.Variant.Primary; text: "Replace"; onClicked: root.aiApply("replace") }
                                LogosButton { visible: !!root.aiResult && root.aiResult.how !== "cursor"; enabled: parent.intact; compact: true; variant: root.aiResult && root.aiResult.how === "below" ? LogosButton.Variant.Primary : LogosButton.Variant.Secondary; text: "Insert below"; onClicked: root.aiApply("below") }
                                LogosButton { compact: true; variant: root.aiResult && root.aiResult.how === "cursor" ? LogosButton.Variant.Primary : LogosButton.Variant.Secondary; text: "Insert at cursor"; onClicked: root.aiApply("cursor") }
                                LogosButton { compact: true; text: "Copy"; onClicked: root.copy(root.aiResult.text, "AI text") }
                                LogosButton { compact: true; text: "Discard"; onClicked: root.aiResult = null }
                            }
                        }
                    }

                    // visual: the document as a page you type into
                    Card {
                        visible: root.visualEdit
                        color: root.paperBg
                        border.color: root.paperBorder
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        ScrollView {
                            id: richScroll
                            anchors.fill: parent
                            anchors.margins: 1
                            clip: true
                            TextArea {
                                id: richArea
                                readonly property int side: Math.max(28, (richScroll.width - 760) / 2)
                                textFormat: TextEdit.RichText
                                wrapMode: TextEdit.Wrap
                                selectByMouse: true
                                persistentSelection: true
                                color: root.paperText
                                selectionColor: Theme.colors.getColor(Theme.palette.primary, 0.35)
                                selectedTextColor: root.paperText
                                placeholderTextColor: root.paperFaint
                                palette.link: root.paperLink
                                font.family: Theme.typography.publicSans
                                font.pixelSize: 16
                                leftPadding: side
                                rightPadding: side
                                topPadding: 28
                                bottomPadding: 60
                                placeholderText: "Start writing… (# heading, - list, [] task, > quote)"
                                background: Item {}
                                onTextChanged: { root.applyPendingFont(); root.inlineRule(); root.editorChanged() }
                                onCursorPositionChanged: { root.updateBlockKind(); root.cursorMoved() }
                                // others' carets (shared drafts)
                                Repeater {
                                    model: root.isShared ? root.collabOnline : []
                                    Item {
                                        readonly property int at: { root.collab; richArea.length; return root.peerCaretPos(modelData.caret) }
                                        readonly property rect r: at >= 0 ? richArea.positionToRectangle(at) : Qt.rect(0, 0, 0, 0)
                                        visible: at >= 0 && modelData.caret !== null && modelData.caret !== undefined
                                        x: r.x; y: r.y; width: 2; height: Math.max(r.height, 14); z: 5
                                        Rectangle { anchors.fill: parent; color: modelData.color }
                                        Rectangle {
                                            y: -14; width: peerName.implicitWidth + 8; height: 14; radius: 3; color: modelData.color
                                            LogosText { id: peerName; x: 4; anchors.verticalCenter: parent.verticalCenter; textFormat: Text.PlainText; font.pixelSize: 10; color: "white"; text: modelData.name }
                                        }
                                    }
                                }

                                // Ctrl+click opens a link (a plain click puts the cursor in it to edit)
                                onReleased: function (ev) {
                                    if (!(ev.modifiers & Qt.ControlModifier)) return
                                    // (linkAt wants other coordinates in a padded TextArea; the hovered link is the one clicked)
                                    var l = String(richArea.hoveredLink || richArea.linkAt(ev.x - richArea.leftPadding, ev.y - richArea.topPadding))
                                    if (l) root.openLink(l)
                                }
                                HoverHandler { cursorShape: richArea.hoveredLink ? Qt.PointingHandCursor : Qt.IBeamCursor }
                                ToolTip.visible: richArea.hoveredLink !== ""
                                ToolTip.delay: 600
                                ToolTip.text: "Ctrl+click to open the link"
                                // Basecamp binds Ctrl+K (its search), and Tab moves the focus: take them
                                // while the editor has focus
                                Keys.onShortcutOverride: function (e) {
                                    if ((e.modifiers & Qt.ControlModifier) && (e.key === Qt.Key_K || e.key === Qt.Key_B || e.key === Qt.Key_I || e.key === Qt.Key_Z || e.key === Qt.Key_Y)) e.accepted = true
                                    if (e.key === Qt.Key_Tab || e.key === Qt.Key_Backtab) e.accepted = true
                                }
                                Keys.onTabPressed: function (e) { root.editorTab(false); e.accepted = true }
                                Keys.onBacktabPressed: function (e) { root.editorTab(true); e.accepted = true }
                                Keys.onPressed: function (e) {
                                    var ctrl = e.modifiers & Qt.ControlModifier
                                    // a word ends a step: the space or Enter after it starts the next one
                                    if (!ctrl && (e.key === Qt.Key_Space || e.key === Qt.Key_Return || e.key === Qt.Key_Enter)) root.commitStep()
                                    if (ctrl && e.key === Qt.Key_Z) { if (e.modifiers & Qt.ShiftModifier) root.redoStep(); else root.undoStep(); e.accepted = true }
                                    else if (ctrl && e.key === Qt.Key_Y) { root.redoStep(); e.accepted = true }
                                    else if (ctrl && e.key === Qt.Key_B) { root.toggleFont("bold"); e.accepted = true }
                                    else if (ctrl && e.key === Qt.Key_I) { root.toggleFont("italic"); e.accepted = true }
                                    else if (ctrl && e.key === Qt.Key_K) { root.askLink(); e.accepted = true }
                                    else if (!ctrl && e.key === Qt.Key_Space && root.typedShortcut()) e.accepted = true
                                    // Tab nests a list item, Shift+Tab lifts it; in code it indents. It
                                    // never moves the focus out of the editor.
                                    else if (e.key === Qt.Key_Tab || e.key === Qt.Key_Backtab) {
                                        root.editorTab(e.key === Qt.Key_Backtab || (e.modifiers & Qt.ShiftModifier))
                                        e.accepted = true
                                    }
                                    // Enter on an empty list item or quote line ends the list or quote; on
                                    // an empty last line of a code block it leaves the code block
                                    // (the block kind the editor keeps up to date, not a fresh ask of the
                                    // core: a call here lets the next typed key in before this Enter)
                                    else if ((e.key === Qt.Key_Return || e.key === Qt.Key_Enter) && !(e.modifiers & Qt.ShiftModifier)
                                             && root.isBlank(root.blockText())
                                             && (/^(ul|ol|task|done|quote)$/.test(root.blockKind) || (root.blockKind === "code" && root.kindAfter() !== "code"))) {
                                        // a nested item goes up one level, a top-level one leaves the list
                                        root.setBlock(/^(ul|ol|task|done)$/.test(root.blockKind) ? "outdent" : "p")
                                        e.accepted = true
                                    }
                                    // Enter at the end of a heading starts normal text, as in Fileverse
                                    else if ((e.key === Qt.Key_Return || e.key === Qt.Key_Enter) && !(e.modifiers & Qt.ShiftModifier)) {
                                        // decided before Qt splits the block: is this a heading, and are we at its end?
                                        var p = richArea.getText(0, richArea.length), c = richArea.cursorPosition
                                        var atEnd = c === p.length || root.pieceBreak.test(p.charAt(c))
                                        if (/^h[1-3]$/.test(root.blockKind) && atEnd) Qt.callLater(function () { root.setBlock("p") })
                                    }
                                }
                            }
                        }
                    }

                    // markdown source + preview
                    SplitView {
                        visible: !root.visualEdit
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        orientation: Qt.Horizontal
                        ScrollView {
                            SplitView.preferredWidth: parent.width / 2
                            SplitView.minimumWidth: 200
                            TextArea {
                                id: mdArea
                                // the selection stays while you write to the AI chat (it says what "this" is)
                                persistentSelection: true
                                color: root.paperText
                                font.pixelSize: 14
                                font.family: Theme.typography.mono
                                wrapMode: TextEdit.Wrap
                                selectByMouse: true
                                textFormat: TextEdit.PlainText
                                padding: 14
                                background: Rectangle { color: root.paperInset; radius: Theme.spacing.radiusLarge; border.color: root.paperBorder }
                                onTextChanged: if (activeFocus) root.editorChanged()
                                // Tab indents (nested lists are four spaces), never leaves the editor
                                Keys.onShortcutOverride: function (e) {
                                    if (e.key === Qt.Key_Tab || e.key === Qt.Key_Backtab) e.accepted = true
                                    if ((e.modifiers & Qt.ControlModifier) && (e.key === Qt.Key_Z || e.key === Qt.Key_Y)) e.accepted = true
                                }
                                Keys.onPressed: function (e) {
                                    if (!(e.modifiers & Qt.ControlModifier)) {
                                        if (e.key === Qt.Key_Space || e.key === Qt.Key_Return || e.key === Qt.Key_Enter) root.commitStep()
                                        return
                                    }
                                    if (e.key === Qt.Key_Z) { if (e.modifiers & Qt.ShiftModifier) root.redoStep(); else root.undoStep(); e.accepted = true }
                                    else if (e.key === Qt.Key_Y) { root.redoStep(); e.accepted = true }
                                }
                                Keys.onTabPressed: function (e) { mdArea.insert(mdArea.cursorPosition, "    "); e.accepted = true }
                                Keys.onBacktabPressed: function (e) {
                                    var t = mdArea.text, c = mdArea.cursorPosition, s0 = t.lastIndexOf("\n", c - 1) + 1
                                    var n = 0; while (n < 4 && t.charAt(s0 + n) === " ") n++
                                    if (n) mdArea.remove(s0, s0 + n)
                                    e.accepted = true
                                }
                                onCursorPositionChanged: root.cursorMoved()
                                // others' carets (shared drafts)
                                Repeater {
                                    model: root.isShared ? root.collabOnline : []
                                    Item {
                                        readonly property int at: { root.collab; mdArea.length; return root.peerCaretPos(modelData.caret) }
                                        readonly property rect r: at >= 0 ? mdArea.positionToRectangle(at) : Qt.rect(0, 0, 0, 0)
                                        visible: at >= 0 && modelData.caret !== null && modelData.caret !== undefined
                                        x: r.x; y: r.y; width: 2; height: Math.max(r.height, 14); z: 5
                                        Rectangle { anchors.fill: parent; color: modelData.color }
                                        Rectangle {
                                            y: -14; width: peerName.implicitWidth + 8; height: 14; radius: 3; color: modelData.color
                                            LogosText { id: peerName; x: 4; anchors.verticalCenter: parent.verticalCenter; textFormat: Text.PlainText; font.pixelSize: 10; color: "white"; text: modelData.name }
                                        }
                                    }
                                }

                            }
                        }
                        Rectangle {
                            SplitView.fillWidth: true
                            color: root.paperBg
                            radius: Theme.spacing.radiusLarge
                            border.color: root.paperBorder
                            ListView {
                                id: previewList
                                anchors.fill: parent
                                anchors.margins: 16
                                clip: true
                                spacing: 8
                                model: root.mode === "edit" && !root.visualEdit ? root.blocks : []
                                ScrollBar.vertical: ScrollBar {}
                                delegate: Loader {
                                    width: previewList.width - 12
                                    property var b: modelData
                                    property real fs: 1
                                    sourceComponent: root.blockComponent(b)
                                }
                            }
                        }
                    }
                }

                // AI chat
                Rectangle {
                    visible: root.chatOpen
                    Layout.preferredWidth: 380
                    Layout.fillHeight: true
                    radius: Theme.spacing.radiusLarge
                    color: Theme.palette.backgroundSecondary
                    border.color: Theme.palette.borderHairline
                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 8
                        RowLayout {
                            Layout.fillWidth: true
                            LogosText { Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 15; font.weight: Theme.typography.weightBold; text: "💬 AI chat" }
                            LogosButton { compact: true; text: "New chat"; enabled: root.chatMsgs.length > 0 && !root.chatBusy; onClicked: root.chatClear() }
                            LogosButton { compact: true; text: "✕"; onClicked: root.chatToggle() }
                        }
                        Hint { text: "About this document. The AI sees it and what changes in it, and can propose edits; you decide what goes in. The chat stays on this device." }
                        ListView {
                            id: chatList
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            spacing: 8
                            model: root.chatMsgs
                            onCountChanged: { Qt.callLater(function () { chatList.positionViewAtEnd() }); chatEndAgain.restart() }
                            // once more after the new message and the closing stream bubble have their heights
                            Timer { id: chatEndAgain; interval: 80; onTriggered: chatList.positionViewAtEnd() }
                            // the answer as it streams in, under the last message
                            footer: Item {
                                width: chatList.width
                                height: partialBox.visible ? partialBox.height + 8 : 0
                            Rectangle {
                                y: 8
                                id: partialBox
                                visible: root.chatBusy && root.chatPartial !== ""
                                width: chatList.width
                                readonly property real cap: Math.max(160, chatList.height * 0.6)
                                height: visible ? Math.min(partialText.implicitHeight + 14, cap) : 0
                                radius: 10; clip: true
                                color: Theme.palette.backgroundInset
                                Flickable {
                                    id: partialFlick
                                    anchors.fill: parent
                                    anchors.margins: 7
                                    contentWidth: width
                                    contentHeight: partialText.implicitHeight
                                    clip: true
                                    boundsBehavior: Flickable.StopAtBounds
                                    // follow the newest text, unless the reader scrolled up
                                    property bool follow: true
                                    onMovementEnded: follow = contentY >= contentHeight - height - 4
                                    onContentHeightChanged: if (follow) contentY = Math.max(0, contentHeight - height)
                                    TextEdit {
                                        id: partialText
                                        x: 3; width: partialFlick.width - 6
                                        readOnly: true; wrapMode: TextEdit.Wrap; textFormat: TextEdit.MarkdownText
                                        font.pixelSize: 13; font.family: Theme.typography.publicSans; color: Theme.palette.text
                                        text: root.chatPartial
                                    }
                                }
                                onVisibleChanged: if (visible) partialFlick.follow = true
                            }
                            }
                            Connections {
                                target: root
                                function onChatPartialChanged() { Qt.callLater(function () { chatList.positionViewAtEnd() }) }
                            }
                            delegate: Column {
                                id: msgItem
                                required property var modelData
                                required property int index
                                width: chatList.width
                                spacing: 4
                                // the document went to the AI: a small marker
                                LogosText {
                                    visible: msgItem.modelData.role === "doc"
                                    width: parent.width; horizontalAlignment: Text.AlignHCenter
                                    textFormat: Text.PlainText; font.pixelSize: 11; color: Theme.palette.textTertiary
                                    text: msgItem.modelData.full ? "📄 the document went to the AI" : "📄 the changes in the document went to the AI"
                                }
                                Rectangle {
                                    visible: msgItem.modelData.role === "user" || msgItem.modelData.role === "assistant"
                                    readonly property bool mine: msgItem.modelData.role === "user"
                                    width: Math.min(parent.width - 24, bubbleText.implicitWidth + 20)
                                    x: mine ? parent.width - width : 0
                                    height: bubbleText.implicitHeight + 14
                                    radius: 10
                                    color: mine ? Theme.colors.getColor(Theme.palette.primary, 0.22) : Theme.palette.backgroundInset
                                    TextEdit {
                                        id: bubbleText
                                        x: 10; y: 7
                                        width: Math.min(implicitWidth, chatList.width - 44)
                                        readOnly: true; selectByMouse: true; wrapMode: TextEdit.Wrap
                                        textFormat: parent.mine ? TextEdit.PlainText : TextEdit.MarkdownText
                                        font.pixelSize: 13; font.family: Theme.typography.publicSans
                                        color: Theme.palette.text
                                        text: msgItem.modelData.text || (msgItem.modelData.edits && msgItem.modelData.edits.length ? "Proposed edits:" : "")
                                    }
                                }
                                LogosText {
                                    visible: msgItem.modelData.role === "assistant" && !!(msgItem.modelData.model || msgItem.modelData.privacy)
                                    textFormat: Text.PlainText; font.pixelSize: 10; color: Theme.palette.textTertiary
                                    text: (msgItem.modelData.model || "") + (msgItem.modelData.connector ? " · " + msgItem.modelData.connector : "") + (msgItem.modelData.privacy ? " · " + msgItem.modelData.privacy : "")
                                }
                                // the AI's proposed edits
                                Repeater {
                                    model: msgItem.modelData.role === "assistant" ? (msgItem.modelData.edits || []) : []
                                    Rectangle {
                                        id: editCard
                                        required property var modelData
                                        required property int index
                                        width: msgItem.width
                                        height: editCol.implicitHeight + 16
                                        radius: 8
                                        color: Theme.palette.background
                                        border.color: editCard.modelData.state === "applied" ? Theme.palette.success : editCard.modelData.state === "new" ? Theme.colors.getColor(Theme.palette.primary, 0.6) : Theme.palette.borderHairline
                                        Column {
                                            id: editCol
                                            x: 8; y: 8; width: parent.width - 16
                                            spacing: 4
                                            LogosText {
                                                width: parent.width; visible: editCard.modelData.find !== ""
                                                textFormat: Text.PlainText; font.pixelSize: 12; font.strikeout: true; color: Theme.palette.textTertiary
                                                wrapMode: Text.Wrap; maximumLineCount: 6; elide: Text.ElideRight
                                                text: editCard.modelData.find
                                            }
                                            LogosText {
                                                width: parent.width
                                                textFormat: Text.PlainText; font.pixelSize: 12; color: Theme.palette.text
                                                wrapMode: Text.Wrap; maximumLineCount: 12; elide: Text.ElideRight
                                                text: editCard.modelData.replace !== "" ? editCard.modelData.replace : "(delete)"
                                            }
                                            LogosText {
                                                width: parent.width; visible: !!editCard.modelData.why
                                                textFormat: Text.PlainText; font.pixelSize: 11; font.italic: true; color: Theme.palette.textSecondary
                                                wrapMode: Text.Wrap
                                                text: editCard.modelData.why || ""
                                            }
                                            LogosButton {
                                                visible: editCard.modelData.state === "missing" || editCard.modelData.state === "ambiguous"
                                                compact: true; text: "Copy new text"
                                                onClicked: root.copy(editCard.modelData.replace, "New text")
                                            }
                                            Row {
                                                spacing: 6
                                                // a card that did not fit can be tried again (the matching got looser in 0.18.4)
                                                visible: editCard.modelData.state === "new" || editCard.modelData.state === "missing"
                                                LogosButton { compact: true; variant: LogosButton.Variant.Primary; text: "Apply"; onClicked: root.chatApply(msgItem.index, editCard.index) }
                                                LogosButton { compact: true; text: "Dismiss"; onClicked: root.chatSetEdit(msgItem.index, editCard.index, "dismissed") }
                                            }
                                            LogosText {
                                                visible: editCard.modelData.state !== "new"
                                                width: parent.width; wrapMode: Text.Wrap
                                                textFormat: Text.PlainText; font.pixelSize: 11
                                                color: editCard.modelData.state === "missing" || editCard.modelData.state === "ambiguous" ? Theme.palette.warning : Theme.palette.textTertiary
                                                text: editCard.modelData.state === "applied" ? "✓ Applied (Ctrl+Z undoes it)"
                                                    : editCard.modelData.state === "missing" ? "Could not find this text in the document. Copy the new text and paste it yourself, or ask again."
                                                    : editCard.modelData.state === "ambiguous" ? "This text is in the document more than once. Copy the new text and paste it where it belongs, or ask again with more context."
                                                    : "Dismissed"
                                            }
                                        }
                                    }
                                }
                                LogosButton {
                                    visible: msgItem.modelData.role === "assistant" && (msgItem.modelData.edits || []).filter(function (e) { return e.state === "new" }).length > 1
                                    compact: true; text: "Apply all"
                                    onClicked: root.chatApplyAll(msgItem.index)
                                }
                            }
                        }
                        Hint { visible: root.chatBusy && root.chatPartial === ""; text: root.chatSession ? "The AI is thinking…" : "Waiting for AI Gateway… (it asks you once to allow this chat)" }
                        Hint {
                            visible: !!root.chatSession && !root.chatBusy
                            text: "Chat allowed in AI Gateway" + (root.chatSession && root.chatSession.model ? " · " + root.chatSession.model : "") + (root.chatSession && root.chatSession.privacy ? " · " + root.chatSession.privacy : "") + ". Closing the chat ends it."
                        }
                        Hint { visible: !root.chatBusy && root.chatNote !== ""; color: Theme.palette.warning; text: root.chatNote }
                        Hint { visible: !root.chatBusy && root.chatMsgs.length === 0 && root.chatNote === ""; text: "Try: “What is unclear in this document?” or “Add a short introduction.”" }
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 6
                            ScrollView {
                                Layout.fillWidth: true
                                Layout.preferredHeight: Math.min(Math.max(chatInput.implicitHeight, 36), 120)
                                TextArea {
                                    id: chatInput
                                    placeholderText: "Ask about the document or ask for a change…"
                                    wrapMode: TextEdit.Wrap
                                    font.pixelSize: 13; font.family: Theme.typography.publicSans
                                    color: Theme.palette.text
                                    background: Rectangle { radius: 8; color: Theme.palette.backgroundInset; border.color: chatInput.activeFocus ? Theme.palette.primary : Theme.palette.borderHairline }
                                    // Enter sends, Shift+Enter is a new line
                                    Keys.onReturnPressed: function (e) {
                                        if (e.modifiers & Qt.ShiftModifier) { e.accepted = false; return }
                                        e.accepted = true
                                        if (root.chatSend(chatInput.text)) chatInput.text = ""
                                    }
                                }
                            }
                            LogosButton { compact: true; variant: LogosButton.Variant.Primary; enabled: !root.chatBusy && chatInput.text.trim() !== ""; text: "Send"; onClicked: { if (root.chatSend(chatInput.text)) chatInput.text = "" } }
                        }
                    }
                }
            }

            // tabs (docs) / sheet tabs
            Flow {
                Layout.fillWidth: true
                spacing: 6
                visible: !root.showSettings && (root.mode === "doc" && (root.tabs.length > 1 || root.sheets.length > 1))
                Repeater {
                    model: root.tabs.length > 1 ? root.tabs : root.sheets
                    Rectangle {
                        readonly property bool on: root.tabs.length > 1 ? root.tabIndex === index : root.sheetIndex === index
                        implicitWidth: tabLabel.implicitWidth + 24
                        implicitHeight: 30
                        radius: 15
                        color: on ? Theme.colors.getColor(Theme.palette.primary, 0.16) : (tabMouse.containsMouse ? Theme.palette.hover : Theme.palette.backgroundSecondary)
                        border.color: on ? Theme.colors.getColor(Theme.palette.primary, 0.5) : Theme.palette.borderHairline
                        LogosText { id: tabLabel; anchors.centerIn: parent; textFormat: Text.PlainText; font.pixelSize: 12; font.weight: Theme.typography.weightMedium; color: parent.on ? Theme.palette.primary : Theme.palette.textSecondary; text: (root.tabs.length > 1 ? "  ".repeat(modelData.depth || 0) : "") + modelData.name }
                        MouseArea { id: tabMouse; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { if (root.tabs.length > 1) root.tabIndex = index; else root.sheetIndex = index } }
                    }
                }
            }

            // document body: a page of readable width
            Card {
                visible: !root.showSettings && (root.mode === "doc" && root.tabs.length > 0)
                color: root.paperBg
                border.color: root.paperBorder
                Layout.fillWidth: true
                Layout.fillHeight: true
                ListView {
                    id: docList
                    anchors.fill: parent
                    anchors.topMargin: 28
                    anchors.bottomMargin: 8
                    clip: true
                    spacing: 8
                    model: root.mode === "doc" ? root.blocks : []
                    ScrollBar.vertical: ScrollBar {}
                    delegate: Item {
                        width: docList.width
                        height: pageBlock.height
                        Loader {
                            id: pageBlock
                            x: Math.max(28, (docList.width - width) / 2)
                            width: Math.min(760, docList.width - 56)
                            property var b: modelData
                            property real fs: 1
                            sourceComponent: root.blockComponent(b)
                        }
                    }
                }
            }
            // sheet body
            ColumnLayout {
                visible: !root.showSettings && (root.mode === "doc" && root.sheet !== null)
                Layout.fillWidth: true
                Layout.fillHeight: true
                LogosText {
                    visible: root.sheet && root.sheet.truncated
                    textFormat: Text.PlainText
                    font.pixelSize: 12
                    color: Theme.palette.warning
                    text: root.sheet ? ("Showing the first " + root.sheet.rows.length + " rows and " + (root.sheet.rows.length ? root.sheet.rows[0].length : 0) + " columns of " + root.sheet.rowCount + " × " + root.sheet.colCount + ".") : ""
                }
                LogosText {
                    visible: root.sheet && root.sheet.rows.length === 0
                    textFormat: Text.PlainText
                    color: Theme.palette.textTertiary
                    text: "This sheet is empty."
                }
                Card {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    Flickable {
                        anchors.fill: parent
                        anchors.margins: 1
                        clip: true
                        contentWidth: sheetList.width
                        flickableDirection: Flickable.HorizontalFlick
                        ScrollBar.horizontal: ScrollBar {}
                        ListView {
                            id: sheetList
                            height: parent.height
                            width: 48 + 120 * (root.sheet && root.sheet.rows.length ? root.sheet.rows[0].length : 0)
                            clip: true
                            model: root.sheet ? root.sheet.rows.length + 1 : 0
                            ScrollBar.vertical: ScrollBar {}
                            delegate: Row {
                                property int r: index - 1
                                Rectangle {
                                    width: 48; height: 28
                                    color: Theme.palette.surfaceRaised
                                    border.color: Theme.palette.borderHairline
                                    LogosText { anchors.centerIn: parent; textFormat: Text.PlainText; font.pixelSize: 11; color: Theme.palette.textTertiary; text: r < 0 ? "" : String(r + 1) }
                                }
                                Repeater {
                                    model: root.sheet && root.sheet.rows.length ? root.sheet.rows[0].length : 0
                                    Rectangle {
                                        width: 120; height: 28
                                        color: r < 0 ? Theme.palette.surfaceRaised : "transparent"
                                        border.color: Theme.palette.borderHairline
                                        LogosText {
                                            anchors.fill: parent
                                            anchors.margins: 6
                                            verticalAlignment: Text.AlignVCenter
                                            horizontalAlignment: r < 0 ? Text.AlignHCenter : Text.AlignLeft
                                            elide: Text.ElideRight
                                            textFormat: Text.PlainText
                                            font.pixelSize: 12
                                            color: r < 0 ? Theme.palette.textTertiary : Theme.palette.text
                                            text: r < 0 ? root.colName(index) : (root.sheet.rows[r][index] || "")
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Item { visible: root.showSettings || root.mode === "home" || root.mode === "import"; Layout.fillHeight: true; Layout.fillWidth: true }
        }
    }

    // Block renderers. A Loader's own properties are not in scope for the item it
    // loads, so each reads the block (`b`) and font scale (`fs`) through `parent`.
    Component {
        id: textBlock
        Rectangle {
            readonly property var b: parent && parent.b ? parent.b : ({ t: "p", html: "", text: "" })
            readonly property real fs: parent && parent.fs ? parent.fs : 1
            readonly property bool heading: b.t.length === 2 && b.t.charAt(0) === "h"
            readonly property int level: heading ? parseInt(b.t.charAt(1)) : 0
            // lists sit in from the text, with the marker in a hanging margin (as Google Docs
            // and ddocs.new show them), one step further per level
            readonly property bool listItem: b.t === "li" || b.t === "oli" || b.t === "task"
            readonly property real indent: listItem ? (28 + (b.depth || 0) * 24) * fs : (b.depth || 0) * 22 * fs
            width: parent ? parent.width : 0
            height: body.implicitHeight + (b.callout ? 16 : 0)
            color: b.callout ? root.paperCode : "transparent"
            radius: Theme.spacing.radiusSmall
            Rectangle { visible: !!b.quote; width: 3; height: parent.height; color: root.paperBorder }
            Text {
                id: body
                x: (b.quote ? 14 : 0) + (b.callout ? 8 : 0) + indent
                y: b.callout ? 8 : 0
                width: parent.width - x - (b.callout ? 8 : 0)
                wrapMode: Text.Wrap
                textFormat: b.t === "image" || b.t === "embed" ? Text.PlainText : Text.RichText
                color: b.t === "image" || b.t === "embed" ? root.paperFaint : root.paperText
                linkColor: root.paperLink
                font.family: Theme.typography.publicSans
                font.pixelSize: Math.round((heading ? [0, 28, 22, 18, 16, 15, 14][level] : 15) * fs)
                font.bold: heading
                // a little more air above headings and after paragraphs; list items stay close
                topPadding: heading ? (level <= 2 ? 14 : 8) * fs : 0
                bottomPadding: b.t === "p" ? 4 * fs : 0
                text: {
                    if (b.t === "image") return "[image" + (b.text ? ": " + b.text : "") + "]"
                    if (b.t === "embed") return "[embedded: " + b.text + "]"
                    return root.codeRead(root.linkTint(b.html))
                }
                onLinkActivated: function (link) { root.openLink(link) }
                HoverHandler { cursorShape: body.hoveredLink ? Qt.PointingHandCursor : Qt.ArrowCursor }
            }
            Text {
                visible: listItem
                x: body.x - 26 * fs
                y: body.y + body.topPadding
                width: 20 * fs
                horizontalAlignment: Text.AlignRight
                textFormat: Text.PlainText
                font: body.font
                color: root.paperSub
                text: b.t === "oli" ? b.num + "." : b.t === "task" ? (b.checked ? "☑" : "☐") : ((b.depth || 0) % 2 ? "◦" : "•")
            }
        }
    }
    Component {
        id: codeBlock
        Rectangle {
            readonly property var b: parent && parent.b ? parent.b : ({ text: "" })
            readonly property real fs: parent && parent.fs ? parent.fs : 1
            width: parent ? parent.width : 0
            height: code.implicitHeight + 16
            color: root.paperCode
            radius: Theme.spacing.radiusSmall
            Text {
                id: code
                x: 8; y: 8
                width: parent.width - 16
                wrapMode: Text.WrapAnywhere
                textFormat: Text.PlainText
                font.family: Theme.typography.mono
                font.pixelSize: Math.round(13 * fs)
                color: root.paperText
                text: b.text
            }
        }
    }
    Component {
        id: hrBlock
        Item {
            width: parent ? parent.width : 0
            height: 14
            Rectangle { anchors.centerIn: parent; width: parent.width; height: 1; color: root.paperBorder }
        }
    }
    Component {
        id: tableBlock
        Column {
            id: tbl
            readonly property var b: parent && parent.b ? parent.b : ({ rows: [] })
            readonly property real fs: parent && parent.fs ? parent.fs : 1
            readonly property int ncols: b.rows.length ? b.rows[0].length : 1
            width: parent ? parent.width : 0
            Repeater {
                model: tbl.b.rows
                Row {
                    id: trow
                    readonly property var cells: modelData
                    readonly property int ri: index
                    Repeater {
                        model: trow.cells
                        Rectangle {
                            width: tbl.width / Math.max(1, tbl.ncols)
                            height: cellText.implicitHeight + 10
                            color: trow.ri === 0 ? root.paperCode : "transparent"
                            border.color: root.paperBorder
                            Text {
                                id: cellText
                                x: 5; y: 5
                                width: parent.width - 10
                                wrapMode: Text.Wrap
                                textFormat: Text.RichText
                                font.family: Theme.typography.publicSans
                                font.pixelSize: Math.round(13 * tbl.fs)
                                font.bold: trow.ri === 0
                                color: root.paperText
                                linkColor: root.paperLink
                                text: root.codeRead(root.linkTint(modelData))
                                onLinkActivated: function (link) { root.openLink(link) }
                            }
                        }
                    }
                }
            }
        }
    }
    // Images: the core fetches the encrypted image (hash-checked) and decrypts it;
    // the view keeps the result in memory as a data: URI. Nothing is written to disk.
    property var imageCache: ({})          // cid -> data URI, "loading" or "failed"
    property int imageTick: 0
    // The cache is changed in place (no change signal, so bindings that read it cannot
    // loop); `imageTick` tells them when to look again.
    function imageSrc(img) {
        // A draft's own picture has no CID until it is published, only a local reference.
        var key = img ? (img.cid || (img.local ? "local:" + img.local : "")) : ""
        if (!key) return ""
        var c = root.imageCache[key]
        if (c !== undefined) return c
        root.imageCache[key] = "loading"
        var done = function (v) { root.imageCache[key] = v; root.imageTick++ }
        Qt.callLater(function () {
            root.quietJob("image", [JSON.stringify(img), root.viewDir], function (r) { done(r && (r.fileUrl || r.dataUri) ? (r.fileUrl || r.dataUri) : "failed") }, function () { done("failed") })
        })
        return "loading"
    }
    // Basecamp's sandbox lets this view load files only from its own install directory
    // (not data: URLs), so the core writes decrypted images to a short-lived cache there.
    readonly property string viewDir: {
        var u = String(Qt.resolvedUrl("."))
        return u.indexOf("file://") === 0 ? decodeURIComponent(u.substring(7)) : ""
    }
    Component {
        id: imageBlock
        Item {
            id: ib
            readonly property var b: parent && parent.b ? parent.b : ({ t: "image", text: "" })
            readonly property real fs: parent && parent.fs ? parent.fs : 1
            // On a slide the box has a fixed height, so the deck can be measured before
            // the image arrives; in the document it follows the picture.
            readonly property real fixedHeight: parent && parent.imageHeight ? parent.imageHeight : 0
            readonly property string src: { root.imageTick; return root.imageSrc(b.img) }
            readonly property bool ready: src.indexOf("data:") === 0 || src.indexOf("file:") === 0
            readonly property real natural: pic.status === Image.Ready && pic.sourceSize.width > 0 ? width * pic.sourceSize.height / pic.sourceSize.width : 0
            width: parent ? parent.width : 0
            height: fixedHeight > 0 ? fixedHeight : (ready && natural > 0 ? Math.min(natural, Math.max(pic.sourceSize.height, 1), 520 * fs) : 120 * fs)
            Image {
                id: pic
                anchors.fill: parent
                source: ib.ready ? ib.src : ""
                fillMode: Image.PreserveAspectFit
                verticalAlignment: Image.AlignTop
                horizontalAlignment: ib.b.img && ib.b.img.align === "left" ? Image.AlignLeft : ib.b.img && ib.b.img.align === "right" ? Image.AlignRight : Image.AlignHCenter
                asynchronous: true
                smooth: true
                mipmap: true
            }
            Rectangle {
                visible: !ib.ready
                anchors.fill: parent
                radius: Theme.spacing.radiusLarge
                color: root.paperCode
                LogosText {
                    anchors.centerIn: parent
                    width: parent.width - 24
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                    font.pixelSize: Math.round(13 * ib.fs)
                    color: root.paperFaint
                    text: !ib.b.img ? ("Image not available here" + (ib.b.text ? ": " + ib.b.text : ""))
                          : ib.src === "failed" ? "Image could not be loaded" : "Loading image…"
                }
            }
        }
    }
    // ── diagrams: a `mermaid` code block (as dDocs stores them), drawn by the core ──
    function isDiagram(b) { return !!b && b.t === "code" && String(b.lang || "").trim().toLowerCase() === "mermaid" }
    readonly property string diagramColors: JSON.stringify({
        background: String(root.paperBg), surface: String(root.paperCode),
        text: String(root.paperText), textSecondary: String(root.paperSub),
        border: String(root.paperBorder), primary: String(root.paperLink),
        font: String(Theme.typography.publicSans)
    })
    property var diagramCache: ({})   // source -> "loading" | { url, w, h } | { error }
    onDiagramColorsChanged: root.diagramCache = ({})   // drawn again in the paper's colours
    function diagramOf(source) {
        var c = root.diagramCache[source]
        if (c !== undefined) return c
        root.diagramCache[source] = "loading"
        var done = function (v) { root.diagramCache[source] = v; root.imageTick++ }
        Qt.callLater(function () {
            root.quietJob("diagram", [source, root.diagramColors, root.viewDir],
                function (r) { done(r && r.fileUrl ? { url: r.fileUrl, w: r.width, h: r.height } : { error: "no picture" }) },
                function (e) { done({ error: e || "could not be drawn" }) })
        })
        return "loading"
    }
    Component {
        id: diagramBlock
        Item {
            id: db
            readonly property var b: parent && parent.b ? parent.b : ({ t: "code", text: "" })
            readonly property real fs: parent && parent.fs ? parent.fs : 1
            readonly property real fixedHeight: parent && parent.imageHeight ? parent.imageHeight : 0
            readonly property var d: { root.imageTick; return root.diagramOf(b.text) }
            readonly property bool ready: typeof d === "object" && !!d.url
            readonly property bool failed: typeof d === "object" && !!d.error
            // as drawn: no wider than the page, nor much larger than its own size
            readonly property real shownW: ready ? Math.min(width, d.w * 1.4 * fs) : width
            readonly property real natural: ready ? shownW * d.h / Math.max(1, d.w) : 0
            width: parent ? parent.width : 0
            height: failed ? fallback.height : fixedHeight > 0 ? fixedHeight : ready ? Math.min(natural, 640 * fs) : 120 * fs
            Image {
                anchors.fill: parent
                visible: db.ready
                source: db.ready ? db.d.url : ""
                // rasterised from the SVG at the size shown (twice, for sharp text)
                sourceSize.width: Math.max(1, Math.round(Math.min(db.width, db.shownW) * 2))
                fillMode: Image.PreserveAspectFit
                horizontalAlignment: Image.AlignHCenter
                verticalAlignment: db.fixedHeight > 0 ? Image.AlignVCenter : Image.AlignTop
                asynchronous: true
                smooth: true
                mipmap: true
            }
            Rectangle {
                visible: !db.ready && !db.failed
                anchors.fill: parent
                radius: Theme.spacing.radiusLarge
                color: root.paperCode
                LogosText { anchors.centerIn: parent; textFormat: Text.PlainText; font.pixelSize: Math.round(13 * db.fs); color: root.paperFaint; text: "Drawing diagram…" }
            }
            // not drawable here: show its source, and why
            Column {
                id: fallback
                visible: db.failed
                width: parent.width
                spacing: 4
                LogosText {
                    width: parent.width; wrapMode: Text.WordWrap; textFormat: Text.PlainText
                    font.pixelSize: Math.round(12 * db.fs); color: root.paperFaint
                    text: "Diagram not drawn here (" + (db.failed ? db.d.error : "") + "); its source:"
                }
                Loader { width: parent.width; property var b: db.b; property real fs: db.fs; sourceComponent: codeBlock }
            }
        }
    }
    // Side by side (an image beside text). dDocs' layouts: align-right makes the first
    // column twice as wide, align-left the last one.
    Component {
        id: columnsBlock
        Row {
            id: cb
            readonly property var b: parent && parent.b ? parent.b : ({ cols: [] })
            readonly property real fs: parent && parent.fs ? parent.fs : 1
            readonly property real imageHeight: parent && parent.imageHeight ? parent.imageHeight : 0
            readonly property var weights: {
                var n = (b.cols || []).length, w = []
                for (var i = 0; i < n; i++) w.push(1)
                if (n >= 2 && b.layout === "align-right") w[0] = 2
                if (n >= 2 && b.layout === "align-left") w[n - 1] = 2
                return w
            }
            readonly property real total: weights.reduce(function (a, x) { return a + x }, 0) || 1
            width: parent ? parent.width : 0
            spacing: 18 * fs
            Repeater {
                model: cb.b.cols || []
                Column {
                    readonly property var colBlocks: modelData
                    width: (cb.width - cb.spacing * Math.max(0, cb.weights.length - 1)) * cb.weights[index] / cb.total
                    spacing: 8 * cb.fs
                    Repeater {
                        model: parent.colBlocks
                        Loader {
                            width: parent.width
                            property var b: modelData
                            property real fs: cb.fs
                            property real imageHeight: cb.imageHeight
                            sourceComponent: root.blockComponent(b)
                        }
                    }
                }
            }
        }
    }
    function blockComponent(b) {
        if (!b) return textBlock
        if (b.t === "hr" || b.t === "pagebreak") return hrBlock
        if (b.t === "table") return tableBlock
        if (root.isDiagram(b)) return diagramBlock
        if (b.t === "code") return codeBlock
        if (b.t === "image") return imageBlock
        if (b.t === "columns") return columnsBlock
        return textBlock
    }

    // ── presentation ──
    // As dDocs' direct-render presentation (fileverse-ddocs#550): the core cuts the
    // document at its structure (an H1/H2 opens a slide, page breaks end one); here each
    // of those is fitted to a fixed 1080x608 stage by measuring it, breaking between
    // blocks (list items one by one) and never inside columns, tables or images. A
    // heading never ends a slide. The stage is then scaled to the screen.
    readonly property int stageW: 1080
    readonly property int stageH: 608
    readonly property int stagePad: 48
    readonly property real stageContentH: stageH - 2 * stagePad
    property real slideFontScale: 1.0
    readonly property real deckFs: 1.45 * slideFontScale
    readonly property real deckImageHeight: stageContentH * 0.62
    readonly property var coreSlides: root.activeTabs.length > 0 ? (root.activeTabs[Math.min(root.tabIndex, root.activeTabs.length - 1)].slides || []) : []
    property bool presenting: false
    property int slideIndex: 0
    property var deck: []
    readonly property var slide: root.deck.length > 0 ? root.deck[Math.min(root.slideIndex, root.deck.length - 1)] : null

    function isHeadingIdx(i) { var t = root.blocks[i] ? root.blocks[i].t : ""; return t.length === 2 && t.charAt(0) === "h" }
    function heightOf(indices) {
        measRep.model = indices
        measCol.forceLayout()
        return measCol.implicitHeight
    }
    // Break one structural slide into slides that fit the stage.
    function fitSlide(indices) {
        var out = [], pending = indices.slice()
        while (pending.length > 0) {
            if (pending.length === 1 || root.heightOf(pending) <= root.stageContentH) { out.push(pending); break }
            // largest run of whole blocks that fits
            var k = pending.length - 1
            while (k > 1 && root.heightOf(pending.slice(0, k)) > root.stageContentH) k--
            if (k === 1 && root.heightOf(pending.slice(0, 1)) > root.stageContentH) k = 1
            var head = pending.slice(0, k)
            // Only headings fit before a block too tall to share: keep them together and
            // let the slide run long, rather than strand a title on its own.
            if (head.every(root.isHeadingIdx)) {
                k = Math.min(pending.length, k + 1)
            } else {
                // a heading never ends a slide: it moves on with its content
                while (k > 1 && root.isHeadingIdx(pending[k - 1])) k--
            }
            out.push(pending.slice(0, k))
            pending = pending.slice(k)
        }
        return out
    }
    function slideKind(indices) {
        var one = indices.length === 1 ? root.blocks[indices[0]] : null
        if (one && (one.t === "image" || root.isDiagram(one))) return "image"
        return indices.every(root.isHeadingIdx) ? "title" : "content"
    }
    function buildDeck() {
        var keep = root.slide ? root.slide.blocks[0] : -1
        var out = []
        root.coreSlides.forEach(function (s) {
            root.fitSlide(s.blocks).forEach(function (part) { out.push({ kind: root.slideKind(part), blocks: part }) })
        })
        measRep.model = []
        root.deck = out
        // stay on the slide that holds what was on screen
        var at = 0
        for (var i = 0; i < out.length; i++) if (out[i].blocks.indexOf(keep) >= 0) { at = i; break }
        root.slideIndex = keep >= 0 ? at : Math.min(root.slideIndex, Math.max(0, out.length - 1))
    }
    function present() { root.slideIndex = 0; root.deck = []; root.buildDeck(); root.slideIndex = 0; root.presenting = true; deckPane.forceActiveFocus() }
    function nextSlide() { if (root.slideIndex < root.deck.length - 1) root.slideIndex++ }
    function prevSlide() { if (root.slideIndex > 0) root.slideIndex-- }
    function setFontScale(v) {
        root.slideFontScale = Math.max(0.6, Math.min(2.0, Math.round(v * 10) / 10))
        root.buildDeck()
    }
    // Basecamp shows this view inside a widget, whose window cannot go full screen;
    // presenting full screen moves the deck into a window of its own.
    property bool fullScreen: false
    Window {
        id: presWindow
        visible: false
        title: "Presentation"
        // Screen size up front: without a window manager (or if it ignores the
        // full-screen request) the window would otherwise open tiny.
        width: Screen.width > 0 ? Screen.width : 1280
        height: Screen.height > 0 ? Screen.height : 720
        color: root.paper === "basecamp" ? Theme.palette.background : root.paperBg
        onClosing: function (close) { root.leaveFullScreen() }
    }
    function toggleFullScreen() { if (root.fullScreen) root.leaveFullScreen(); else root.enterFullScreen() }
    function enterFullScreen() {
        if (root.fullScreen) return
        deckPane.parent = presWindow.contentItem
        root.fullScreen = true
        presWindow.showFullScreen()
        presWindow.raise()
        presWindow.requestActivate()
        deckPane.forceActiveFocus()
    }
    function leaveFullScreen() {
        if (!root.fullScreen) return
        root.fullScreen = false
        deckPane.parent = root
        presWindow.hide()
        deckPane.forceActiveFocus()
    }
    function findDeck() { return deckPane }
    function stopPresenting() { root.leaveFullScreen(); root.presenting = false }
    // Re-cut the deck when what it shows changes (an update arrived, another tab).
    Connections {
        target: root
        function onCoreSlidesChanged() { if (root.presenting) Qt.callLater(root.buildDeck) }
    }

    // The stage, laid out off screen at full size, for measuring.
    Item {
        x: -20000
        width: root.stageW - 2 * root.stagePad
        height: 10
        opacity: 0
        Column {
            id: measCol
            width: parent.width
            spacing: 10 * root.deckFs
            Repeater {
                id: measRep
                model: []
                Loader {
                    width: measCol.width
                    property var b: root.blocks[modelData]
                    property real fs: root.deckFs
                    property real imageHeight: root.deckImageHeight
                    sourceComponent: root.blockComponent(b)
                }
            }
        }
    }

    Rectangle {
        id: deckPane
        visible: root.presenting && root.slide !== null
        anchors.fill: parent
        color: root.paper === "basecamp" ? Theme.palette.background : root.paperBg
        focus: root.presenting
        Keys.onPressed: function (e) {
            if (e.key === Qt.Key_Right || e.key === Qt.Key_Space || e.key === Qt.Key_PageDown || e.key === Qt.Key_Down) { root.nextSlide(); e.accepted = true }
            else if (e.key === Qt.Key_Left || e.key === Qt.Key_PageUp || e.key === Qt.Key_Up) { root.prevSlide(); e.accepted = true }
            else if (e.key === Qt.Key_Home) { root.slideIndex = 0; e.accepted = true }
            else if (e.key === Qt.Key_End) { root.slideIndex = root.deck.length - 1; e.accepted = true }
            else if (e.key === Qt.Key_Plus || e.key === Qt.Key_Equal) { root.setFontScale(root.slideFontScale + 0.1); e.accepted = true }
            else if (e.key === Qt.Key_Minus) { root.setFontScale(root.slideFontScale - 0.1); e.accepted = true }
            else if (e.key === Qt.Key_0) { root.setFontScale(1.0); e.accepted = true }
            else if (e.key === Qt.Key_F) { root.toggleFullScreen(); e.accepted = true }
            else if (e.key === Qt.Key_Escape) {
                if (root.fullScreen) root.leaveFullScreen()
                else root.presenting = false
                e.accepted = true
            }
        }
        MouseArea { anchors.fill: parent; onClicked: function (m) { if (m.x > width / 3) root.nextSlide(); else root.prevSlide(); deckPane.forceActiveFocus() } }

        // the stage, at its real size, scaled to the screen
        Item {
            id: stage
            width: root.stageW
            height: root.stageH
            anchors.centerIn: parent
            anchors.verticalCenterOffset: controls.visible ? -26 : 0
            scale: Math.min((deckPane.width - 40) / root.stageW, (deckPane.height - (controls.visible ? 96 : 24)) / root.stageH)
            Rectangle {
                anchors.fill: parent
                radius: 10
                color: Theme.palette.surface
                border.color: Theme.palette.borderHairline
            }
            Item {
                id: slideArea
                x: root.stagePad
                y: root.stagePad
                width: root.stageW - 2 * root.stagePad
                height: root.stageContentH
                clip: true
                Column {
                    id: slideCol
                    readonly property string kind: root.slide ? root.slide.kind : ""
                    readonly property real fsHere: kind === "title" ? root.deckFs * 1.4 : root.deckFs
                    // A slide the measurement let run long (a title kept with a large table)
                    // shrinks to fit rather than being cut off.
                    readonly property real fit: Math.min(1, slideArea.height / Math.max(1, implicitHeight))
                    width: parent.width
                    y: kind === "title" || kind === "image" ? (slideArea.height - implicitHeight * fit) / 2 : 0
                    scale: fit
                    transformOrigin: Item.TopLeft
                    spacing: 10 * root.deckFs
                    Repeater {
                        model: root.slide ? root.slide.blocks : []
                        Loader {
                            width: slideCol.width
                            property var b: root.blocks[modelData]
                            property real fs: slideCol.fsHere
                            property real imageHeight: slideCol.kind === "image" ? root.stageContentH : root.deckImageHeight
                            sourceComponent: root.blockComponent(b)
                        }
                    }
                }
            }
            // slide number
            LogosText {
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.rightMargin: 22
                anchors.bottomMargin: 14
                textFormat: Text.PlainText
                font.pixelSize: 15
                color: Theme.palette.textTertiary
                text: (root.slideIndex + 1) + " / " + root.deck.length
            }
        }

        RowLayout {
            id: controls
            visible: !root.fullScreen
            anchors.bottom: parent.bottom
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottomMargin: 16
            spacing: Theme.spacing.small
            LogosButton { compact: true; text: "‹ Prev"; enabled: root.slideIndex > 0; onClicked: { root.prevSlide(); deckPane.forceActiveFocus() } }
            LogosText { textFormat: Text.PlainText; color: Theme.palette.textTertiary; text: (root.slideIndex + 1) + " / " + root.deck.length }
            LogosButton { compact: true; text: "Next ›"; enabled: root.slideIndex < root.deck.length - 1; onClicked: { root.nextSlide(); deckPane.forceActiveFocus() } }
            Item { width: 12 }
            LogosButton { compact: true; text: "A−"; onClicked: { root.setFontScale(root.slideFontScale - 0.1); deckPane.forceActiveFocus() } }
            LogosText { textFormat: Text.PlainText; color: Theme.palette.textTertiary; text: Math.round(root.slideFontScale * 100) + "%" }
            LogosButton { compact: true; text: "A+"; onClicked: { root.setFontScale(root.slideFontScale + 0.1); deckPane.forceActiveFocus() } }
            Item { width: 12 }
            LogosButton { compact: true; text: "Full screen (F)"; onClicked: { root.toggleFullScreen(); deckPane.forceActiveFocus() } }
            LogosButton { compact: true; text: "Exit (Esc)"; onClicked: root.stopPresenting() }
        }
    }

    // toast
    Rectangle {
        visible: root.toast !== ""
        anchors.bottom: parent.bottom
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottomMargin: 24
        width: Math.min(parent.width - 80, toastRow.implicitWidth + 36)
        height: Math.max(40, toastText.implicitHeight + 18)
        radius: 20
        color: Theme.palette.surfaceRaised
        border.color: Theme.palette.borderSubtle
        RowLayout {
            id: toastRow
            anchors.centerIn: parent
            width: Math.min(implicitWidth, parent.width - 36)
            spacing: 8
            LogosIcon { Layout.preferredWidth: 14; Layout.preferredHeight: 14; source: LogosIcons.check; color: Theme.palette.success }
            LogosText { id: toastText; Layout.maximumWidth: root.width - 160; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 13; text: root.toast }
        }
    }

    // editing an AI prompt (Settings → AI prompts, My AI actions)
    Rectangle {
        anchors.fill: parent
        visible: root.promptEdit !== null
        z: 999
        color: Qt.rgba(0, 0, 0, 0.55)
        MouseArea { anchors.fill: parent }   // a click outside does nothing: the text would be lost
        onVisibleChanged: if (visible) {
            promptText.text = root.promptEdit.text
            promptName.text = root.promptEdit.name || ""
            root.promptHow = root.promptEdit.how || "replace"
            if (root.promptEdit.mine !== undefined && root.promptEdit.mine < 0) promptName.forceActiveFocus(); else promptText.forceActiveFocus()
        }
        Rectangle {
            anchors.centerIn: parent
            width: Math.min(640, parent.width - 60)
            height: Math.min(promptCol.implicitHeight + 40, parent.height - 60)
            radius: Theme.spacing.radiusLarge
            color: Theme.palette.surfaceRaised
            border.color: Theme.palette.borderSubtle
            MouseArea { anchors.fill: parent }
            ColumnLayout {
                id: promptCol
                readonly property bool mine: root.promptEdit !== null && root.promptEdit.mine !== undefined
                anchors { left: parent.left; right: parent.right; top: parent.top; margins: 20 }
                spacing: 10
                LogosText { Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 16; font.weight: Theme.typography.weightBold
                            text: !root.promptEdit ? "" : promptCol.mine ? (root.promptEdit.mine < 0 ? "New AI action" : "Edit AI action") : root.promptEdit.title }
                Field { id: promptName; visible: promptCol.mine; Layout.fillWidth: true; maximumLength: 40; placeholderText: "Button name, e.g. Friendlier" }
                Hint {
                    text: !root.promptEdit ? "" : promptCol.mine ? "What to ask the AI. It gets this with the selection, or the paragraph the cursor is in."
                          : root.promptEdit.key === "system" ? "Sent with every Write with AI action: how answers should look. Fileverse puts the answer in your document as it comes, so ask for the text only."
                          : root.promptEdit.key === "translate" ? "{language} is replaced with the language you type."
                          : root.promptEdit.key === "chat" ? "The start of the AI chat's instructions. Fileverse adds after it where the document is and how to propose edits; that part stays, because the edit cards depend on it. Applies from the next AI chat you open."
                          : "What to ask the AI. It gets this with the selection, or the paragraph the cursor is in."
                }
                ScrollView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 200
                    TextArea {
                        id: promptText
                        wrapMode: TextEdit.Wrap
                        font.family: Theme.typography.publicSans
                        selectByMouse: true
                        color: Theme.palette.text
                        font.pixelSize: 13
                        background: Rectangle { color: Theme.palette.backgroundInset; radius: Theme.spacing.radiusLarge; border.color: promptText.activeFocus ? Theme.palette.primary : Theme.palette.borderSubtle }
                    }
                }
                RowLayout {
                    visible: promptCol.mine
                    spacing: Theme.spacing.small
                    LogosText { textFormat: Text.PlainText; font.pixelSize: 12; color: Theme.palette.textSecondary; text: "The answer:" }
                    Repeater {
                        model: [{ k: "replace", t: "Replaces the text" }, { k: "below", t: "Goes below" }, { k: "cursor", t: "Continues at the cursor" }]
                        LogosButton { compact: true; text: modelData.t
                                      variant: root.promptHow === modelData.k ? LogosButton.Variant.Primary : LogosButton.Variant.Secondary
                                      onClicked: root.promptHow = modelData.k }
                    }
                }
                Hint { visible: promptCol.mine && root.promptHow === "cursor"; text: "Works on the text before the cursor, like Continue writing." }
                Hint { visible: promptCol.mine && root.promptHow === "below"; text: "Reads the whole document when nothing is selected, like Summarize." }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacing.small
                    LogosButton {
                        compact: true; visible: !promptCol.mine; text: "Reset to default"
                        enabled: !!root.promptEdit && promptText.text !== (root.promptEdit.def || "")
                        onClicked: promptText.text = root.promptEdit.def
                    }
                    Item { Layout.fillWidth: true }
                    LogosButton { compact: true; text: "Cancel"; onClicked: root.promptEdit = null }
                    LogosButton {
                        compact: true; variant: LogosButton.Variant.Primary; text: "Save"
                        enabled: promptText.text.trim() !== "" && (!promptCol.mine || promptName.text.trim() !== "")
                        onClicked: {
                            var e = root.promptEdit
                            if (promptCol.mine) { if (root.aiSetMine(e.mine, promptName.text, promptText.text, root.promptHow)) root.promptEdit = null }
                            else { root.aiSetPrompt(e.key, e.def, promptText.text); root.promptEdit = null }
                        }
                    }
                }
            }
        }
    }

    // confirmation before removing something
    Rectangle {
        anchors.fill: parent
        visible: root.confirmAsk !== null
        z: 1000
        color: Qt.rgba(0, 0, 0, 0.55)
        MouseArea { anchors.fill: parent; onClicked: root.confirmAsk = null }   // a click outside cancels
        Keys.onEscapePressed: root.confirmAsk = null
        onVisibleChanged: if (visible) confirmCancel.forceActiveFocus()
        Rectangle {
            anchors.centerIn: parent
            width: Math.min(460, parent.width - 60)
            height: confirmCol.implicitHeight + 40
            radius: Theme.spacing.radiusLarge
            color: Theme.palette.surfaceRaised
            border.color: Theme.palette.borderSubtle
            MouseArea { anchors.fill: parent }   // clicks inside do not cancel
            ColumnLayout {
                id: confirmCol
                anchors { left: parent.left; right: parent.right; top: parent.top; margins: 20 }
                spacing: 12
                LogosText { Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 16; font.weight: Theme.typography.weightBold
                            text: root.confirmAsk ? root.confirmAsk.title : "" }
                LogosText { Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 13; color: Theme.palette.textSecondary
                            text: root.confirmAsk ? root.confirmAsk.text : "" }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacing.small
                    Item { Layout.fillWidth: true }
                    LogosButton { id: confirmCancel; compact: true; text: "Cancel"; onClicked: root.confirmAsk = null }
                    LogosButton {
                        compact: true; variant: LogosButton.Variant.Primary
                        text: root.confirmAsk ? root.confirmAsk.button : ""
                        onClicked: { var run = root.confirmAsk.run; root.confirmAsk = null; run() }
                    }
                }
            }
        }
    }

    // Basecamp hosts this view in a QQuickWidget, which hands Tab to the view only when
    // some item after the focused one can take focus; otherwise the focus leaves the view
    // and the editor never sees the key. This invisible stop keeps Tab in the view, where
    // the editors use it (lists nest, code indents); if it ever gets the focus it gives it
    // back to the editor.
    Item {
        id: tabStop
        width: 0; height: 0
        activeFocusOnTab: true
        onActiveFocusChanged: if (activeFocus && root.mode === "edit") (root.visualEdit ? richArea : mdArea).forceActiveFocus()
    }
}
