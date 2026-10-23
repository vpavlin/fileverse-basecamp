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
    function elapsed(label) {
        for (var k in root.jobs) if (root.jobs[k].label === label) return Math.round((Date.now() - root.jobs[k].started) / 1000)
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
        root.portal = p; root.portalFiles = []; root.portalError = ""; root.mode = "portal"
        root.startJob("list_portal", [p], "list", function (r) { root.portalFiles = r.files }, function (e) { root.portalError = e })
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

    component Field: TextField {
        color: Theme.palette.text
        placeholderTextColor: Theme.palette.textTertiary
        font.pixelSize: 14
        font.family: Theme.typography.publicSans
        leftPadding: 8
        rightPadding: 8
        background: Rectangle {
            color: Theme.palette.surface
            radius: Theme.spacing.radiusSmall
            border.color: parent.activeFocus ? Theme.palette.primary : Theme.palette.border
        }
    }
    component Hint: LogosText {
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
        textFormat: Text.PlainText
        font.pixelSize: 12
        color: Theme.palette.textTertiary
    }
    component SideItem: Rectangle {
        id: si
        property string label: ""
        property string sub: ""
        property bool active: false
        property bool removable: false
        signal clicked()
        signal removed()
        Layout.fillWidth: true
        implicitHeight: sub === "" ? 30 : 44
        radius: Theme.spacing.radiusSmall
        color: active ? Theme.palette.surfaceRaised : (siMouse.containsMouse ? Theme.palette.surface : "transparent")
        MouseArea { id: siMouse; anchors.fill: parent; hoverEnabled: true; onClicked: si.clicked() }
        ColumnLayout {
            anchors.fill: parent; anchors.margins: 6; anchors.rightMargin: si.removable ? 24 : 6; spacing: 0
            LogosText { Layout.fillWidth: true; elide: Text.ElideRight; textFormat: Text.PlainText; font.pixelSize: 13; text: si.label }
            LogosText { visible: si.sub !== ""; Layout.fillWidth: true; elide: Text.ElideRight; textFormat: Text.PlainText; font.pixelSize: 11; color: Theme.palette.textTertiary; text: si.sub }
        }
        LogosText {
            visible: si.removable
            anchors.right: parent.right; anchors.rightMargin: 8; anchors.verticalCenter: parent.verticalCenter
            text: "×"; textFormat: Text.PlainText; font.pixelSize: 16; color: Theme.palette.textTertiary
            MouseArea { anchors.fill: parent; anchors.margins: -6; onClicked: si.removed() }
        }
    }

    readonly property var tabs: root.docv && root.docv.tabs ? root.docv.tabs : []
    readonly property var draftTabs: root.draft && root.draft.preview && root.draft.preview.length !== undefined ? root.draft.preview : []
    readonly property var activeTabs: root.mode === "edit" ? root.draftTabs : root.tabs
    readonly property var sheets: root.mode === "doc" && root.docv && root.docv.sheets ? root.docv.sheets : []
    readonly property var blocks: root.activeTabs.length > 0 ? root.activeTabs[Math.min(root.tabIndex, root.activeTabs.length - 1)].blocks : []
    readonly property var sheet: root.sheets.length > 0 ? root.sheets[Math.min(root.sheetIndex, root.sheets.length - 1)] : null

    function shortId(s) { s = String(s || ""); return s.length > 14 ? s.slice(0, 8) + "…" + s.slice(-4) : s }
    function colName(i) { var s = ""; i += 1; while (i > 0) { var m = (i - 1) % 26; s = String.fromCharCode(65 + m) + s; i = Math.floor((i - 1) / 26) } return s }
    function when(ms) { return ms ? new Date(ms).toLocaleString() : "" }
    function kindIcon(k, layout) { return k === "dsheet" ? "▦ " : (layout === "logos" ? "✎ " : "¶ ") }

    Rectangle { anchors.fill: parent; color: Theme.palette.background }

    RowLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing.large
        spacing: Theme.spacing.large

        // ── sidebar ──
        ColumnLayout {
            Layout.preferredWidth: 250
            Layout.minimumWidth: 250
            Layout.maximumWidth: 250
            Layout.fillHeight: true
            spacing: Theme.spacing.small

            LogosText { text: "Fileverse"; font.pixelSize: 22; font.bold: true; textFormat: Text.PlainText }
            Hint { text: "No Fileverse server: Gnosis via eth_rpc_module, hash-checked IPFS blocks, your own node and Logos Storage." }
            RowLayout {
                Layout.fillWidth: true
                spacing: 4
                Repeater {
                    model: [["recent", "Recent"], ["files", "My files"], ["drafts", "Drafts"]]
                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 30
                        radius: Theme.spacing.radiusSmall
                        color: root.section === modelData[0] ? Theme.palette.surfaceRaised : "transparent"
                        border.color: root.section === modelData[0] ? Theme.palette.primary : Theme.palette.border
                        LogosText { anchors.centerIn: parent; textFormat: Text.PlainText; font.pixelSize: 13; text: modelData[1] }
                        MouseArea { anchors.fill: parent; onClicked: root.section = modelData[0] }
                    }
                }
            }

            // Recent
            ScrollView {
                visible: root.section === "recent"
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                ColumnLayout {
                    width: 250; spacing: 2
                    Repeater {
                        model: root.snap.recent || []
                        SideItem {
                            label: root.kindIcon(modelData.kind, modelData.layout) + (modelData.title || "Untitled")
                            sub: (modelData.layout === "logos" ? "published here" : (modelData.url.indexOf("fvown://") === 0 ? "own file #" : root.shortId(modelData.portal) + " #") + (modelData.layout === "logos" ? "" : modelData.fileId))
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

            // My files (backup keys)
            ScrollView {
                visible: root.section === "files"
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                ColumnLayout {
                    width: 250; spacing: 2
                    Repeater {
                        model: root.snap.portals || []
                        SideItem {
                            label: "Portal " + root.shortId(modelData.portal)
                            sub: (modelData.kind === "legacy" ? "legacy (RSA)" : (modelData.kind === "pq" ? "post-quantum only" : "current")) + (modelData.source ? " · " + modelData.source : "")
                            active: root.mode === "portal" && root.portal === modelData.portal
                            removable: true
                            onClicked: root.showPortal(modelData.portal)
                            onRemoved: root.act("forget_portal", [modelData.portal], "Keys removed from this device")
                        }
                    }
                    LogosButton { Layout.fillWidth: true; text: "Import backup key…"; onClicked: root.mode = "import" }
                }
            }

            // Drafts
            ScrollView {
                visible: root.section === "drafts"
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                ColumnLayout {
                    width: 250; spacing: 2
                    LogosButton { Layout.fillWidth: true; text: "New document"; onClicked: root.newDraft("Untitled", "# Untitled\n\n", "") }
                    Repeater {
                        model: root.snap.drafts || []
                        SideItem {
                            label: "✎ " + (modelData.title || "Untitled")
                            sub: (modelData.link ? "published " + root.when(modelData.publishedAt) : "draft · " + root.when(modelData.updatedAt))
                            active: root.mode === "edit" && root.draft && root.draft.id === modelData.id
                            removable: true
                            onClicked: root.openDraft(modelData.id)
                            onRemoved: root.act("draft_delete", [modelData.id], "Draft deleted", function () { if (root.draft && root.draft.id === modelData.id) { root.draft = null; root.mode = "home" } })
                        }
                    }
                }
            }

            LogosButton { text: root.showSettings ? "Close settings" : "Settings"; onClicked: { root.showSettings = !root.showSettings; gwField.text = (root.snap.gateways || []).join("\n"); root.refreshNode(); root.refreshStorage() } }
            LogosText {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                font.pixelSize: 11
                color: root.coreReachable ? Theme.palette.textTertiary : Theme.palette.warning
                text: root.coreReachable ? ("fileverse_core " + (root.snap.version || "?") + (root.node.reachable ? " · IPFS: " + root.node.source : "") + (root.storage.available ? " · Storage" : "")) : "fileverse_core is not answering"
            }
        }

        // ── main pane ──
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.spacing.medium

            RowLayout {
                Layout.fillWidth: true
                visible: root.mode !== "edit"
                spacing: Theme.spacing.small
                Field {
                    id: linkField
                    Layout.fillWidth: true
                    placeholderText: "Paste a dDocs/dSheets share link, or a fvbc:// link published from Basecamp"
                    onAccepted: root.open(text)
                }
                LogosButton { text: root.busyWith("open") ? ("Opening… " + root.elapsed("open") + "s") : "Open"; enabled: !root.busyWith("open"); onClicked: root.open(linkField.text) }
            }

            // settings
            ColumnLayout {
                visible: root.showSettings
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                LogosText { text: "IPFS gateways, tried after your own node (one per line). Every block is checked against its hash, so a gateway can fail but cannot change content."; wrapMode: Text.WordWrap; Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 12 }
                ScrollView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 80
                    TextArea {
                        id: gwField
                        color: Theme.palette.text
                        font.pixelSize: 13
                        font.family: Theme.typography.mono
                        background: Rectangle { color: Theme.palette.surface; radius: Theme.spacing.radiusSmall; border.color: Theme.palette.border }
                    }
                }
                RowLayout {
                    LogosButton {
                        text: "Save gateways"
                        onClicked: {
                            var list = gwField.text.split("\n").map(function (s) { return s.trim() }).filter(function (s) { return s !== "" })
                            root.act("set_gateways", [JSON.stringify(list)], "Gateways saved")
                        }
                    }
                    LogosButton { text: "Restore defaults"; onClicked: root.act("set_gateways", ["[]"], "Defaults restored", function (r) { gwField.text = r.gateways.join("\n") }) }
                }
                LogosText {
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 12
                    color: root.node.reachable ? Theme.palette.success : Theme.palette.textTertiary
                    text: root.node.reachable
                          ? ("Your IPFS node: " + (root.node.source === "ipfs_module" ? "the IPFS Node module" : "configured below") + " · " + root.shortId(root.node.peerId) + ". Read from first; Keep and Publish pin there.")
                          : "No IPFS node. Install the IPFS Node module (recommended), or run Kubo and enter its API and gateway below."
                }
                RowLayout {
                    Layout.fillWidth: true
                    Field { id: nodeApiField; Layout.fillWidth: true; placeholderText: "Kubo API, e.g. http://127.0.0.1:5001"; text: root.snap.node ? root.snap.node.api : "" }
                    Field { id: nodeGwField; Layout.fillWidth: true; placeholderText: "Kubo gateway, e.g. http://127.0.0.1:8080"; text: root.snap.node ? root.snap.node.gateway : "" }
                    LogosButton { text: "Save node"; onClicked: root.act("set_ipfs_node", [nodeApiField.text, nodeGwField.text], "IPFS node saved", function () { root.refreshNode() }) }
                }
                LogosText {
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 12
                    color: root.storage.available ? Theme.palette.success : Theme.palette.textTertiary
                    text: root.storage.available ? ("Logos Storage: available (" + (root.snap.storedBlocks || 0) + " blocks copied there). Read from when IPFS has nothing.") : "Logos Storage is not available in this Basecamp."
                }
                Hint { text: "Gnosis RPC: " + (root.snap.rpc || "eth_rpc_module") + ". Change the endpoint (or a proxy/Tor) in the Ethereum RPC app." }
            }

            LogosText {
                visible: root.error !== ""
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                textFormat: Text.PlainText
                color: Theme.palette.warning
                text: root.error
                MouseArea { anchors.fill: parent; onClicked: root.error = "" }
            }

            // home
            ColumnLayout {
                visible: root.mode === "home"
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                Hint { font.pixelSize: 14; text: "Open a share link above, open your own files with a backup key (My files), or write a document (Drafts). Keys never leave this device." }
            }

            // import a backup key
            ColumnLayout {
                visible: root.mode === "import"
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                LogosText { text: "Your own files"; font.pixelSize: 20; font.bold: true; textFormat: Text.PlainText }
                Hint { font.pixelSize: 13; text: "dDocs and dSheets let you download backup keys (Settings → Backup). With that file, this app lists and opens every file of your portal straight from Gnosis and IPFS, with no share links and no Fileverse service. The keys are stored only on this device (file mode 600). Files locked with Fileverse's post-quantum scheme cannot be opened yet: its library is not public." }
                RowLayout {
                    LogosButton { text: "Choose backup file…"; onClicked: backupDialog.open() }
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
                        background: Rectangle { color: Theme.palette.surface; radius: Theme.spacing.radiusSmall; border.color: Theme.palette.border }
                    }
                }
                LogosButton {
                    text: "Import"
                    onClicked: root.act("import_backup", [backupText.text], "Backup key imported", function (r) { backupText.text = ""; root.section = "files"; if (r.portals.length) root.showPortal(r.portals[0].portal) })
                }
            }

            // a portal's files
            ColumnLayout {
                visible: root.mode === "portal"
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Theme.spacing.small
                LogosText { text: "Portal " + root.portal; font.pixelSize: 16; font.bold: true; textFormat: Text.PlainText; elide: Text.ElideMiddle; Layout.fillWidth: true }
                Hint { text: root.busyWith("list") ? ("Reading the portal on Gnosis and the files' metadata from IPFS… " + root.elapsed("list") + "s") : (root.portalError !== "" ? "Could not read this portal." : (root.portalFiles.length + " files, newest first.")) }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 2
                    model: root.portalFiles
                    ScrollBar.vertical: ScrollBar {}
                    delegate: SideItem {
                        width: ListView.view.width
                        label: root.kindIcon(modelData.kind, "") + (modelData.title || (modelData.error ? "(cannot open)" : "Untitled"))
                        sub: "#" + modelData.fileId + (modelData.updatedAt ? " · " + root.when(modelData.updatedAt) : "") + (modelData.error ? " · " + modelData.error : "")
                        onClicked: if (!modelData.error) root.open(modelData.url)
                    }
                }
            }

            // document header
            ColumnLayout {
                visible: root.mode === "doc" && root.docv !== null
                Layout.fillWidth: true
                spacing: 2
                LogosText { Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 24; font.bold: true; text: root.docv ? (root.docv.title || "Untitled") : "" }
                LogosText {
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    font.pixelSize: 12
                    color: Theme.palette.textTertiary
                    elide: Text.ElideRight
                    text: !root.docv ? "" : ((root.docv.kind === "dsheet" ? "dSheet" : "dDoc") + " · "
                          + (root.docv.layout === "logos" ? "published from Basecamp (no chain record)" : (root.docv.layout + " portal " + root.shortId(root.docv.portal) + " · file " + root.docv.fileId))
                          + (root.docv.updatedAt ? " · updated " + root.when(root.docv.updatedAt) : ""))
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.spacing.small
                    LogosButton { visible: root.deck.length > 0; text: "Present"; onClicked: root.present() }
                    LogosButton { visible: root.docv && root.docv.markdown !== undefined; text: "Edit a copy"; onClicked: root.newDraft(root.docv.title || "Untitled", root.docv.markdownBody !== undefined ? root.docv.markdownBody : root.docv.markdown, root.docUrl) }
                    LogosButton {
                        visible: root.node.reachable || root.kept
                        enabled: root.node.reachable && !root.busyWith("keep")
                        text: root.busyWith("keep") ? "Pinning…" : (root.kept ? "On my IPFS ✓" : "Keep on my IPFS")
                        onClicked: root.keepCopy()
                    }
                    LogosButton {
                        visible: root.storage.available || root.inStorage
                        enabled: root.storage.available && !root.busyWith("storage")
                        text: root.busyWith("storage") ? ("Copying… " + root.elapsed("storage") + "s") : (root.inStorage ? "In Logos Storage ✓" : "Copy to Logos Storage")
                        onClicked: root.copyToStorage()
                    }
                    LogosButton { visible: root.docv && root.docv.markdown !== undefined; text: "Copy Markdown"; onClicked: root.copy(root.docv.markdown, "Markdown") }
                    LogosButton { visible: root.docUrl.indexOf("fvbc://") === 0; text: "Copy link"; onClicked: root.copy(root.docUrl, "Link") }
                    LogosButton { text: root.showDetails ? "Hide sources" : "Sources"; onClicked: root.showDetails = !root.showDetails }
                }
            }

            // provenance
            ColumnLayout {
                visible: root.mode === "doc" && root.showDetails && root.docv !== null
                Layout.fillWidth: true
                spacing: 2
                Repeater {
                    model: root.docv ? root.docv.provenance.steps : []
                    LogosText { Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 12; text: "• " + modelData }
                }
                Repeater {
                    model: root.docv ? root.docv.provenance.blobs : []
                    LogosText {
                        Layout.fillWidth: true
                        elide: Text.ElideMiddle
                        textFormat: Text.PlainText
                        font.pixelSize: 12
                        color: modelData.verified ? Theme.palette.text : Theme.palette.warning
                        text: (modelData.verified ? "✓ " : "⚠ unverified ") + modelData.cid + " · " + modelData.bytes + " B · " + modelData.gateway
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
                    Field {
                        id: titleField
                        Layout.fillWidth: true
                        font.pixelSize: 18
                        placeholderText: "Title"
                        onTextEdited: { root.draftDirty = true; previewTimer.restart() }
                    }
                    LogosText { textFormat: Text.PlainText; font.pixelSize: 12; color: Theme.palette.textTertiary; text: root.draftDirty ? "editing…" : "saved on this device" }
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.spacing.small
                    LogosButton {
                        enabled: !root.busyWith("publish")
                        text: root.busyWith("publish") ? ("Publishing… " + root.elapsed("publish") + "s") : (root.draft && root.draft.published ? "Publish again" : "Publish")
                        onClicked: root.publishDraft()
                    }
                    LogosButton { visible: root.deck.length > 0; text: "Present"; onClicked: root.present() }
                    LogosButton { visible: !!(root.draft && root.draft.published); text: "Copy link"; onClicked: root.copy(root.draft.published.link, "Link") }
                    LogosButton { visible: !!(root.draft && root.draft.published); text: "Open published"; onClicked: root.open(root.draft.published.link) }
                    LogosButton { visible: !!(root.draft && root.draft.sourceUrl); text: "Open original"; onClicked: root.open(root.draft.sourceUrl) }
                }
                Hint {
                    text: root.draft && root.draft.published
                          ? ("Published " + root.when(root.draft.published.at) + ". Anyone with the link can read it in Basecamp; each publish makes a new link (content-addressed).")
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
                            background: Rectangle { color: Theme.palette.surface; radius: Theme.spacing.radiusSmall; border.color: Theme.palette.border }
                            onTextChanged: if (activeFocus) { root.draftDirty = true; previewTimer.restart() }
                        }
                    }
                    ListView {
                        id: previewList
                        SplitView.fillWidth: true
                        clip: true
                        spacing: 6
                        leftMargin: 12
                        model: root.mode === "edit" ? root.blocks : []
                        ScrollBar.vertical: ScrollBar {}
                        delegate: Loader {
                            width: previewList.width - 28
                            property var b: modelData
                            property real fs: 1
                            sourceComponent: root.blockComponent(b)
                        }
                    }
                }
            }

            // tabs (docs) / sheet tabs
            Flow {
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                visible: root.mode === "doc" && (root.tabs.length > 1 || root.sheets.length > 1)
                Repeater {
                    model: root.tabs.length > 1 ? root.tabs : root.sheets
                    LogosButton {
                        text: (root.tabs.length > 1 ? "  ".repeat(modelData.depth || 0) : "") + modelData.name
                        onClicked: { if (root.tabs.length > 1) root.tabIndex = index; else root.sheetIndex = index }
                    }
                }
            }

            // document body
            ListView {
                id: docList
                visible: root.mode === "doc" && root.tabs.length > 0
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 6
                model: root.mode === "doc" ? root.blocks : []
                ScrollBar.vertical: ScrollBar {}
                delegate: Loader {
                    width: docList.width - 16
                    property var b: modelData
                    property real fs: 1
                    sourceComponent: root.blockComponent(b)
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
                Flickable {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
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
                                width: 48; height: 26
                                color: Theme.palette.surfaceRaised
                                border.color: Theme.palette.borderHairline
                                LogosText { anchors.centerIn: parent; textFormat: Text.PlainText; font.pixelSize: 11; color: Theme.palette.textTertiary; text: r < 0 ? "" : String(r + 1) }
                            }
                            Repeater {
                                model: root.sheet && root.sheet.rows.length ? root.sheet.rows[0].length : 0
                                Rectangle {
                                    width: 120; height: 26
                                    color: r < 0 ? Theme.palette.surfaceRaised : Theme.palette.surface
                                    border.color: Theme.palette.borderHairline
                                    LogosText {
                                        anchors.fill: parent
                                        anchors.margins: 4
                                        verticalAlignment: Text.AlignVCenter
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

    // toast
    Rectangle {
        visible: root.toast !== ""
        anchors.bottom: parent.bottom
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottomMargin: 24
        width: Math.min(parent.width - 80, toastText.implicitWidth + 32)
        height: toastText.implicitHeight + 16
        radius: 18
        color: Theme.palette.surfaceRaised
        LogosText { id: toastText; anchors.centerIn: parent; width: Math.min(implicitWidth, root.width - 112); wrapMode: Text.WordWrap; textFormat: Text.PlainText; text: root.toast }
    }
}
