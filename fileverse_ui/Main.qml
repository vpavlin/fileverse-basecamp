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
    property int tabIndex: 0
    property int sheetIndex: 0
    property bool showDetails: false
    property bool showSettings: false
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
            root.docv = doc; root.docUrl = u; root.mode = "doc"
            root.presenting = false; root.slideIndex = 0; root.tabIndex = 0; root.sheetIndex = 0
            root.refreshNode(); root.refreshStorage()
        })
    }
    readonly property var recentEntry: {
        var rs = root.snap.recent || []
        for (var i = 0; i < rs.length; i++) if (rs[i].url === root.docUrl) return rs[i]
        return null
    }
    // Pinned on the node in use now (a copy on some other node does not count).
    readonly property bool kept: root.recentEntry !== null && !!root.recentEntry.kept && !!root.node.peerId && root.recentEntry.keptOn === root.node.peerId
    readonly property bool inStorage: root.recentEntry !== null && !!root.recentEntry.inStorage
    function keepCopy() {
        root.startJob("keep", [root.docUrl], "keep", function (r) { root.say("Pinned on your IPFS node (" + r.node + "): " + r.blocks + " blocks") })
    }
    function copyToStorage() {
        root.startJob("copy_to_storage", [root.docUrl], "storage", function (r) { root.say("In Logos Storage: " + r.copied + " copied, " + r.alreadyThere + " already there") })
    }
    function refreshNode() { root.core("node_status", [], function (raw) { var r = root.j(raw); if (r && r.ok) root.node = r }) }
    function refreshStorage() { root.core("storage_status", [], function (raw) { var r = root.j(raw); if (r && r.ok) root.storage = r }) }
    function showPortal(p) {
        var changed = root.portal !== p
        root.portal = p; root.portalError = ""; root.mode = "portal"
        if (changed) { root.portalFiles = []; root.portalBackup = ({}); root.portalListedAt = null }
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
        root.startJob("backup_portal", [p], "backup", function (r) {
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
        root.draft = d; root.draftDirty = false; root.mode = "edit"; root.tabIndex = 0
        titleField.text = d.title; mdArea.text = d.markdown
    }
    function openDraft(id) { root.act("draft_get", [id], "", function (r) { root.editDraft(r.draft) }) }
    function saveDraft(then) {
        if (!root.draft) return
        root.core("draft_save", [root.draft.id, titleField.text, mdArea.text], function (raw) {
            var r = root.j(raw)
            if (r && r.ok) { root.draft = r.draft; root.draftDirty = false; root.refresh(); if (then) then() }
            else root.fail(r ? r.error : "Saving failed.")
        })
    }
    function publishDraft() {
        root.saveDraft(function () {
            root.startJob("publish", [root.draft.id], "publish", function (r) {
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
    function quietJob(method, args, onDone) {
        root.core(method, args, function (raw) {
            var r = root.j(raw)
            if (!r || !r.ok || !r.jobId) return
            var q = root.quietJobs; q[r.jobId] = onDone; root.quietJobs = q
        })
    }
    function pollQuiet() {
        Object.keys(root.quietJobs).forEach(function (id) {
            root.core("job", [id], function (raw) {
                var r = root.j(raw)
                if (r && r.ok && r.state === "pending") return
                var cb = root.quietJobs[id]
                var q = root.quietJobs; delete q[id]; root.quietJobs = q
                if (r && r.ok && r.state === "done" && cb) cb(r.result)
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
    Timer { id: previewTimer; interval: 700; onTriggered: root.saveDraft() }

    property bool snapBusy: false
    property bool snapAgain: false
    function refresh() {
        if (root.snapBusy) { root.snapAgain = true; return }
        root.snapBusy = true
        root.core("snapshot", [], function (raw) {
            var s = root.j(raw)
            if (s && s.ok) { root.snap = s; root.missedPolls = 0; root.coreReachable = true }
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
    readonly property var shownPortalFiles: {
        var q = root.portalFilter.trim().toLowerCase()
        if (q === "") return root.portalFiles
        return root.portalFiles.filter(function (f) { return String(f.title || "").toLowerCase().indexOf(q) >= 0 || String(f.fileId) === q })
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
                NavItem { label: "My files"; icon: LogosIcons.pages; count: (root.snap.portals || []).length; active: root.section === "files"; onClicked: root.section = "files" }
                NavItem { label: "Drafts"; icon: LogosIcons.copy; count: (root.snap.drafts || []).length; active: root.section === "drafts"; onClicked: root.section = "drafts" }

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
                                    sub: (modelData.layout === "logos" ? "Published here" : (modelData.url.indexOf("fvown://") === 0 ? "My files" : "Shared link"))
                                         + (modelData.openedAt ? " · " + root.ago(modelData.openedAt) : "")
                                         + (modelData.kept && modelData.keptOn === root.node.peerId ? " · IPFS" : "") + (modelData.inStorage ? " · Storage" : "")
                                    active: root.mode === "doc" && root.docUrl === modelData.url
                                    removable: true
                                    onClicked: root.open(modelData.url)
                                    onRemoved: root.act("forget", [modelData.url], "")
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
                                    onRemoved: root.act("forget_portal", [modelData.portal], "Keys removed from this device")
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
                            LogosButton { Layout.fillWidth: true; Layout.bottomMargin: 6; compact: true; variant: LogosButton.Variant.Primary; text: "New document"; onClicked: root.newDraft("Untitled", "# Untitled\n\n", "") }
                            Repeater {
                                model: root.snap.drafts || []
                                FileRow {
                                    compact: true
                                    kind: "ddoc"; layout: "logos"
                                    title: modelData.title || "Untitled"
                                    sub: (modelData.link ? "Published " + root.ago(modelData.publishedAt) : "Draft · " + root.ago(modelData.updatedAt))
                                    active: root.mode === "edit" && root.draft && root.draft.id === modelData.id
                                    removable: true
                                    onClicked: root.openDraft(modelData.id)
                                    onRemoved: root.act("draft_delete", [modelData.id], "Draft deleted", function () { if (root.draft && root.draft.id === modelData.id) { root.draft = null; root.mode = "home" } })
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
                                ["IPFS node", !!root.node.reachable, root.node.reachable ? (root.node.peers !== undefined ? root.node.peers + " peers" : "on") : "off"],
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

            // settings
            Card {
                visible: root.showSettings
                Layout.fillWidth: true
                implicitHeight: settingsCol.implicitHeight + 32
                ColumnLayout {
                    id: settingsCol
                    anchors.fill: parent
                    anchors.margins: 16
                    spacing: Theme.spacing.small
                    LogosText { textFormat: Text.PlainText; font.pixelSize: 16; font.weight: Theme.typography.weightBold; text: "Settings" }
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
                    LogosText {
                        Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 12
                        color: root.node.reachable ? Theme.palette.success : Theme.palette.textTertiary
                        text: root.node.reachable
                              ? ((root.node.source === "ipfs_module" ? "The IPFS Node module" : "Your Kubo") + " · " + root.shortId(root.node.peerId)
                                 + (root.node.peers !== undefined ? " · " + root.node.peers + " peers · " + root.node.pins + " pinned · " + Math.round((root.node.repoSize || 0) / 1048576) + " MB" : "")
                                 + ". Read from first; Keep, Back up and Publish pin there.")
                              : "No IPFS node. Install the IPFS Node module (recommended), or run Kubo and enter its API and gateway below."
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
                visible: root.mode === "import"
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
                visible: root.mode === "portal"
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
                                text: root.shortId(root.portal)
                                tint: Theme.palette.textSecondary
                                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.copy(root.portal, "Portal address") }
                            }
                            Hint {
                                Layout.fillWidth: false
                                text: root.portalError !== "" && root.portalFiles.length === 0 ? "Could not read this portal."
                                      : (root.portalFiles.length + " files · " + Object.keys(root.portalBackup).length + " backed up"
                                         + (root.portalListedAt ? " · listed " + root.ago(root.portalListedAt) : ""))
                            }
                        }
                    }
                    Item { Layout.fillWidth: true }
                    LogosButton {
                        compact: true
                        enabled: !root.busyWith("list")
                        text: root.busyWith("list") ? ("Refreshing… " + (root.progressOf("list") || root.elapsed("list") + "s")) : "Refresh"
                        onClicked: root.refreshPortal()
                    }
                    LogosButton {
                        compact: true
                        variant: LogosButton.Variant.Primary
                        enabled: !root.busyWith("backup") && root.portalFiles.length > 0
                        text: root.busyWith("backup") ? ("Backing up… " + (root.progressOf("backup") || root.elapsed("backup") + "s")) : "Back up everything"
                        onClicked: root.backupPortal()
                    }
                }
                Field {
                    Layout.fillWidth: true
                    visible: root.portalFiles.length > 6
                    placeholderText: "Filter by title or number"
                    onTextChanged: root.portalFilter = text
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
                            readonly property var bk: root.portalBackup[String(modelData.fileId)]
                            kind: modelData.kind || "ddoc"
                            dim: !!modelData.error
                            title: modelData.title || (modelData.error ? "Cannot open" : "Untitled")
                            sub: "#" + modelData.fileId + (modelData.kind === "dsheet" ? " · Sheet" : " · Doc")
                                 + (modelData.updatedAt ? " · " + root.ago(modelData.updatedAt) : "")
                                 + (modelData.titleFromText ? " · title from text" : "")
                                 + (modelData.error ? " · " + modelData.error : "")
                            chips: root.backupChips(bk)
                            onClicked: if (!modelData.error) root.open(modelData.url)
                        }
                    }
                    Hint {
                        anchors.centerIn: parent
                        width: parent.width - 60
                        horizontalAlignment: Text.AlignHCenter
                        visible: root.shownPortalFiles.length === 0
                        text: root.busyWith("list") ? "Reading the portal on Gnosis…" : (root.portalFilter !== "" ? "No file matches." : "No files listed yet. Press Refresh.")
                    }
                }
                Hint { text: "Back up everything keeps every file's encrypted blocks on this device, pins them on your IPFS node and copies them into Logos Storage, so they stay readable without Fileverse." }
            }

            // document header
            RowLayout {
                visible: root.mode === "doc" && root.docv !== null
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
                visible: root.mode === "doc" && root.docv !== null
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                LogosButton { visible: root.deck.length > 0; compact: true; variant: LogosButton.Variant.Primary; text: "Present"; onClicked: root.present() }
                LogosButton { visible: root.docv && root.docv.markdown !== undefined; compact: true; text: "Edit a copy"; onClicked: root.newDraft(root.docv.title || "Untitled", root.docv.markdownBody !== undefined ? root.docv.markdownBody : root.docv.markdown, root.docUrl) }
                LogosButton {
                    visible: (root.node.reachable || root.kept) && !root.kept
                    compact: true
                    enabled: root.node.reachable && !root.busyWith("keep")
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
                LogosButton { visible: root.docUrl.indexOf("fvbc://") === 0; compact: true; text: "Copy link"; onClicked: root.copy(root.docUrl, "Link") }
                LogosButton { compact: true; text: root.showDetails ? "Hide sources" : "Sources"; onClicked: root.showDetails = !root.showDetails }
            }

            // provenance
            Card {
                visible: root.mode === "doc" && root.showDetails && root.docv !== null
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

            // editor
            ColumnLayout {
                visible: root.mode === "edit" && root.draft !== null
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Theme.spacing.small
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacing.small
                    Field {
                        id: titleField
                        Layout.fillWidth: true
                        font.pixelSize: 20
                        font.weight: Theme.typography.weightBold
                        placeholderText: "Title"
                        background: Item {}
                        onTextEdited: { root.draftDirty = true; previewTimer.restart() }
                    }
                    Chip { text: root.draftDirty ? "editing…" : "saved on this device"; tint: root.draftDirty ? Theme.palette.warning : Theme.palette.success }
                    LogosButton { visible: root.deck.length > 0; compact: true; text: "Present"; onClicked: root.present() }
                    LogosButton {
                        compact: true
                        variant: LogosButton.Variant.Primary
                        enabled: !root.busyWith("publish")
                        text: root.busyWith("publish") ? ("Publishing… " + root.elapsed("publish") + "s") : (root.draft && root.draft.published ? "Publish again" : "Publish")
                        onClicked: root.publishDraft()
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    visible: !!(root.draft && (root.draft.published || root.draft.sourceUrl))
                    spacing: Theme.spacing.small
                    LogosButton { visible: !!(root.draft && root.draft.published); compact: true; text: "Copy link"; onClicked: root.copy(root.draft.published.link, "Link") }
                    LogosButton { visible: !!(root.draft && root.draft.published); compact: true; text: "Open published"; onClicked: root.open(root.draft.published.link) }
                    LogosButton { visible: !!(root.draft && root.draft.sourceUrl); compact: true; text: "Open original"; onClicked: root.open(root.draft.sourceUrl) }
                    Item { Layout.fillWidth: true }
                }
                Hint {
                    text: root.draft && root.draft.published
                          ? ("Published " + root.ago(root.draft.published.at) + ". Anyone with the link can read it in Basecamp; each publish makes a new link (content-addressed).")
                          : "Markdown on the left, the document as it will look on the right. Publishing encrypts it in the Fileverse format and puts it on your IPFS node and in Logos Storage."
                }
                SplitView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    orientation: Qt.Horizontal
                    ScrollView {
                        SplitView.preferredWidth: parent.width / 2
                        SplitView.minimumWidth: 200
                        TextArea {
                            id: mdArea
                            color: Theme.palette.text
                            font.pixelSize: 14
                            font.family: Theme.typography.mono
                            wrapMode: TextEdit.Wrap
                            selectByMouse: true
                            textFormat: TextEdit.PlainText
                            padding: 14
                            background: Rectangle { color: Theme.palette.backgroundInset; radius: Theme.spacing.radiusLarge; border.color: Theme.palette.borderSubtle }
                            onTextChanged: if (activeFocus) { root.draftDirty = true; previewTimer.restart() }
                        }
                    }
                    Rectangle {
                        SplitView.fillWidth: true
                        color: Theme.palette.backgroundSecondary
                        radius: Theme.spacing.radiusLarge
                        border.color: Theme.palette.borderHairline
                        ListView {
                            id: previewList
                            anchors.fill: parent
                            anchors.margins: 16
                            clip: true
                            spacing: 8
                            model: root.mode === "edit" ? root.blocks : []
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

            // tabs (docs) / sheet tabs
            Flow {
                Layout.fillWidth: true
                spacing: 6
                visible: root.mode === "doc" && (root.tabs.length > 1 || root.sheets.length > 1)
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
                visible: root.mode === "doc" && root.tabs.length > 0
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
                visible: root.mode === "doc" && root.sheet !== null
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
            Item { visible: root.mode === "home" || root.mode === "import"; Layout.fillHeight: true; Layout.fillWidth: true }
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
            width: parent ? parent.width : 0
            height: body.implicitHeight + (b.callout ? 16 : 0)
            color: b.callout ? Theme.palette.surfaceRaised : "transparent"
            radius: Theme.spacing.radiusSmall
            Rectangle { visible: !!b.quote; width: 3; height: parent.height; color: Theme.palette.border }
            Text {
                id: body
                x: (b.quote ? 14 : 0) + (b.callout ? 8 : 0) + (b.depth || 0) * 22 * fs
                y: b.callout ? 8 : 0
                width: parent.width - x - (b.callout ? 8 : 0)
                wrapMode: Text.Wrap
                textFormat: b.t === "image" || b.t === "embed" ? Text.PlainText : Text.StyledText
                color: b.t === "image" || b.t === "embed" ? Theme.palette.textTertiary : Theme.palette.text
                linkColor: Theme.palette.primary
                font.family: Theme.typography.publicSans
                font.pixelSize: Math.round((heading ? [0, 28, 22, 18, 16, 15, 14][level] : 15) * fs)
                font.bold: heading
                topPadding: heading ? 8 * fs : 0
                text: {
                    if (b.t === "image") return "[image" + (b.text ? ": " + b.text : "") + "]"
                    if (b.t === "embed") return "[embedded: " + b.text + "]"
                    var marker = ""
                    if (b.t === "li") marker = "•  "
                    else if (b.t === "oli") marker = b.num + ".  "
                    else if (b.t === "task") marker = (b.checked ? "☑  " : "☐  ")
                    return marker + b.html
                }
                onLinkActivated: function (link) { Qt.openUrlExternally(link) }
                HoverHandler { cursorShape: body.hoveredLink ? Qt.PointingHandCursor : Qt.ArrowCursor }
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
            color: Theme.palette.surfaceRaised
            radius: Theme.spacing.radiusSmall
            Text {
                id: code
                x: 8; y: 8
                width: parent.width - 16
                wrapMode: Text.WrapAnywhere
                textFormat: Text.PlainText
                font.family: Theme.typography.mono
                font.pixelSize: Math.round(13 * fs)
                color: Theme.palette.text
                text: b.text
            }
        }
    }
    Component {
        id: hrBlock
        Item {
            width: parent ? parent.width : 0
            height: 14
            Rectangle { anchors.centerIn: parent; width: parent.width; height: 1; color: Theme.palette.border }
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
                            color: trow.ri === 0 ? Theme.palette.surfaceRaised : "transparent"
                            border.color: Theme.palette.borderHairline
                            Text {
                                id: cellText
                                x: 5; y: 5
                                width: parent.width - 10
                                wrapMode: Text.Wrap
                                textFormat: Text.StyledText
                                font.family: Theme.typography.publicSans
                                font.pixelSize: Math.round(13 * tbl.fs)
                                font.bold: trow.ri === 0
                                color: Theme.palette.text
                                linkColor: Theme.palette.primary
                                text: modelData
                                onLinkActivated: function (link) { Qt.openUrlExternally(link) }
                            }
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
        if (b.t === "code") return codeBlock
        return textBlock
    }

    // ── presentation: the current tab as dDocs would present it ──
    property bool presenting: false
    property int slideIndex: 0
    readonly property var deck: root.activeTabs.length > 0 ? (root.activeTabs[Math.min(root.tabIndex, root.activeTabs.length - 1)].slides || []) : []
    readonly property var slide: root.deck.length > 0 ? root.deck[Math.min(root.slideIndex, root.deck.length - 1)] : null
    function present() { root.slideIndex = 0; root.presenting = true; deckPane.forceActiveFocus() }
    function nextSlide() { if (root.slideIndex < root.deck.length - 1) root.slideIndex++ }
    function prevSlide() { if (root.slideIndex > 0) root.slideIndex-- }

    Rectangle {
        id: deckPane
        visible: root.presenting && root.slide !== null
        anchors.fill: parent
        color: Theme.palette.background
        focus: root.presenting
        Keys.onPressed: function (e) {
            if (e.key === Qt.Key_Right || e.key === Qt.Key_Space || e.key === Qt.Key_PageDown) { root.nextSlide(); e.accepted = true }
            else if (e.key === Qt.Key_Left || e.key === Qt.Key_PageUp) { root.prevSlide(); e.accepted = true }
            else if (e.key === Qt.Key_Home) { root.slideIndex = 0; e.accepted = true }
            else if (e.key === Qt.Key_End) { root.slideIndex = root.deck.length - 1; e.accepted = true }
            else if (e.key === Qt.Key_Escape) { root.presenting = false; e.accepted = true }
        }
        MouseArea { anchors.fill: parent; onClicked: function (m) { if (m.x > width / 3) root.nextSlide(); else root.prevSlide(); deckPane.forceActiveFocus() } }

        readonly property real fs: Math.max(1.2, Math.min(width / 760, height / 520))
        Rectangle {
            id: slideCard
            anchors.centerIn: parent
            anchors.verticalCenterOffset: -30
            width: Math.min(parent.width - 80, (parent.height - 150) * 16 / 9)
            height: width * 9 / 16
            radius: Theme.spacing.radiusSmall
            color: Theme.palette.surface
            border.color: Theme.palette.borderHairline
            clip: true
            Column {
                id: slideCol
                readonly property bool isTitle: root.slide && root.slide.kind === "title"
                // Shrink a slide that would overflow the card (long code, big tables).
                readonly property real fit: Math.min(1, (slideCard.height - 80) / Math.max(1, implicitHeight))
                width: parent.width - 96
                x: 48
                y: isTitle ? (parent.height - height * fit) / 2 : 40
                scale: fit
                transformOrigin: Item.TopLeft
                spacing: 10 * deckPane.fs
                Repeater {
                    model: root.slide ? root.slide.blocks : []
                    Loader {
                        width: slideCol.width
                        property var b: root.blocks[modelData]
                        property real fs: slideCol.isTitle ? deckPane.fs * 1.6 : deckPane.fs
                        sourceComponent: root.blockComponent(b)
                    }
                }
            }
        }
        RowLayout {
            anchors.bottom: parent.bottom
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.bottomMargin: 16
            spacing: Theme.spacing.medium
            LogosButton { text: "‹ Prev"; enabled: root.slideIndex > 0; onClicked: { root.prevSlide(); deckPane.forceActiveFocus() } }
            LogosText { textFormat: Text.PlainText; color: Theme.palette.textTertiary; text: (root.slideIndex + 1) + " / " + root.deck.length }
            LogosButton { text: "Next ›"; enabled: root.slideIndex < root.deck.length - 1; onClicked: { root.nextSlide(); deckPane.forceActiveFocus() } }
            LogosButton { text: "Exit (Esc)"; onClicked: root.presenting = false }
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
}
