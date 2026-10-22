// Fileverse viewer: a pure-QML view over the fileverse_core module.
//
// Paste a dDocs/dSheets share link; the core reads the file record on Gnosis
// (through eth_rpc_module), fetches hash-checked blocks from IPFS gateways and
// decrypts them locally. This view only draws the JSON the core returns.
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

import Logos.Theme
import Logos.Controls

Item {
    id: root
    width: 1000
    height: 700

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

    // ── state ──
    property var snap: ({ recent: [], gateways: [] })
    property bool coreReachable: true
    property int missedPolls: 0
    property string jobId: ""
    property int jobElapsed: 0
    property var docv: null          // the opened document (core JSON)
    property string error: ""
    property string toast: ""
    property int tabIndex: 0
    property int sheetIndex: 0
    property bool showDetails: false
    property bool showSettings: false
    property var node: ({ reachable: false })
    property bool keeping: false
    function refreshNode() { root.core("node_status", [], function (raw) { var r = root.j(raw); if (r && r.ok) root.node = r }) }
    readonly property var recentEntry: {
        if (!root.docv) return null
        var rs = root.snap.recent || []
        for (var i = 0; i < rs.length; i++) if (rs[i].portal === root.docv.portal && rs[i].fileId === root.docv.fileId) return rs[i]
        return null
    }
    readonly property bool kept: root.recentEntry !== null && !!root.recentEntry.kept
    function keepCopy() {
        if (!root.docv || root.keeping) return
        root.keeping = true
        root.core("keep", [root.docv.portal, root.docv.fileId], function (raw) {
            root.keeping = false
            var r = root.j(raw)
            if (r && r.ok) { root.say("Kept on your IPFS node: " + r.blocks + " blocks, " + r.pinned + " pinned"); root.refresh() }
            else root.error = (r && r.error) || "Keeping a copy failed - is your IPFS node running?"
        })
    }
    readonly property bool loading: root.jobId !== ""

    function say(msg) { root.toast = msg; toastTimer.restart() }

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

    function open(url) {
        var u = String(url || "").trim()
        if (u === "" || root.loading) return
        root.error = ""
        root.core("open", [u], function (raw) {
            var r = root.j(raw)
            if (!r) { root.error = "No answer from fileverse_core. Is it installed and loaded?"; return }
            if (!r.ok) { root.error = r.error || "Could not open the link."; return }
            root.jobId = r.jobId
            root.jobElapsed = 0
            pollJob()
        })
    }

    property bool jobBusy: false
    function pollJob() {
        if (root.jobId === "" || root.jobBusy) return
        root.jobBusy = true
        var id = root.jobId
        root.core("job", [id], function (raw) {
            root.jobBusy = false
            var r = root.j(raw)
            if (id !== root.jobId) return
            if (!r) return
            if (!r.ok) { root.jobId = ""; root.error = r.error || "The open job was lost."; return }
            root.jobElapsed = Math.round((r.elapsedMs || 0) / 1000)
            if (r.state === "done") {
                root.jobId = ""
                root.docv = r.result
                root.refreshNode()
                root.presenting = false
                root.slideIndex = 0
                root.tabIndex = 0
                root.sheetIndex = 0
                root.error = ""
                root.refresh()
            } else if (r.state === "failed") {
                root.jobId = ""
                root.error = r.error || "Opening failed."
            }
        })
    }

    Timer { interval: 2500; running: true; repeat: true; onTriggered: root.refresh() }
    Timer { interval: 600; running: root.loading; repeat: true; onTriggered: root.pollJob() }
    Timer { id: toastTimer; interval: 2500; onTriggered: root.toast = "" }

    Connections {
        target: typeof logos !== "undefined" ? logos : null
        ignoreUnknownSignals: true
        function onModuleEventReceived(moduleName, eventName, data) {
            if (moduleName === root.coreName && eventName === "job_changed") root.pollJob()
        }
    }
    Component.onCompleted: {
        if (typeof logos !== "undefined" && logos && logos.onModuleEvent) logos.onModuleEvent(root.coreName, "job_changed")
        root.refresh()
        root.refreshNode()
    }

    // Off-screen editor used only to put text on the clipboard.
    TextEdit { id: clip; visible: false; textFormat: TextEdit.PlainText }
    function copy(text) { clip.text = text; clip.selectAll(); clip.copy(); root.say("Copied") }

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

    readonly property var tabs: root.docv && root.docv.tabs ? root.docv.tabs : []
    readonly property var sheets: root.docv && root.docv.sheets ? root.docv.sheets : []
    readonly property var blocks: root.tabs.length > 0 ? root.tabs[Math.min(root.tabIndex, root.tabs.length - 1)].blocks : []
    readonly property var sheet: root.sheets.length > 0 ? root.sheets[Math.min(root.sheetIndex, root.sheets.length - 1)] : null

    function shortId(s) { s = String(s || ""); return s.length > 14 ? s.slice(0, 8) + "…" + s.slice(-4) : s }
    function colName(i) { var s = ""; i += 1; while (i > 0) { var m = (i - 1) % 26; s = String.fromCharCode(65 + m) + s; i = Math.floor((i - 1) / 26) } return s }

    Rectangle { anchors.fill: parent; color: Theme.palette.background }

    RowLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing.large
        spacing: Theme.spacing.large

        // ── recent documents ──
        ColumnLayout {
            Layout.preferredWidth: 240
            Layout.maximumWidth: 240
            Layout.fillHeight: true
            spacing: Theme.spacing.small

            LogosText { text: "Fileverse"; font.pixelSize: 22; font.bold: true; textFormat: Text.PlainText }
            LogosText {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                textFormat: Text.PlainText
                font.pixelSize: 12
                color: Theme.palette.textTertiary
                text: "Read-only viewer. No Fileverse server: Gnosis via eth_rpc_module, IPFS blocks checked against their hashes, decrypted here."
            }
            Item { height: Theme.spacing.small }
            LogosText { text: "Recent"; font.pixelSize: 14; font.bold: true; textFormat: Text.PlainText }
            ListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 2
                model: root.snap.recent || []
                delegate: Rectangle {
                    width: ListView.view.width
                    height: 46
                    radius: Theme.spacing.radiusSmall
                    color: rowMouse.containsMouse ? Theme.palette.surfaceRaised : "transparent"
                    MouseArea { id: rowMouse; anchors.fill: parent; hoverEnabled: true; onClicked: root.open(modelData.url) }
                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 6
                        anchors.rightMargin: 28
                        spacing: 0
                        LogosText {
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                            textFormat: Text.PlainText
                            font.pixelSize: 13
                            text: (modelData.kind === "dsheet" ? "▦ " : "¶ ") + (modelData.title || "Untitled")
                        }
                        LogosText {
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                            textFormat: Text.PlainText
                            font.pixelSize: 11
                            color: Theme.palette.textTertiary
                            text: root.shortId(modelData.portal) + " #" + modelData.fileId
                        }
                    }
                    LogosText {
                        anchors.right: parent.right
                        anchors.rightMargin: 8
                        anchors.verticalCenter: parent.verticalCenter
                        text: "×"
                        textFormat: Text.PlainText
                        font.pixelSize: 16
                        color: Theme.palette.textTertiary
                        MouseArea {
                            anchors.fill: parent
                            anchors.margins: -6
                            onClicked: root.core("forget", [modelData.portal, modelData.fileId], function () { root.refresh() })
                        }
                    }
                }
                LogosText {
                    visible: (root.snap.recent || []).length === 0
                    width: parent.width
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                    font.pixelSize: 12
                    color: Theme.palette.textTertiary
                    text: "Documents you open appear here. They are re-read from IPFS (or the local block cache) each time."
                }
            }
            LogosButton { text: root.showSettings ? "Close settings" : "Settings"; onClicked: { root.showSettings = !root.showSettings; gwField.text = (root.snap.gateways || []).join("\n") } }
            LogosText {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                font.pixelSize: 11
                color: root.coreReachable ? Theme.palette.textTertiary : Theme.palette.warning
                text: root.coreReachable ? ("fileverse_core " + (root.snap.version || "?")) : "fileverse_core is not answering"
            }
        }

        // ── main pane ──
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.spacing.medium

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                Field {
                    id: linkField
                    Layout.fillWidth: true
                    placeholderText: "Paste a dDocs or dSheets share link (…/0x…/<n>#key=…)"
                    onAccepted: root.open(text)
                }
                LogosButton { text: root.loading ? ("Opening… " + root.jobElapsed + "s") : "Open"; enabled: !root.loading; onClicked: root.open(linkField.text) }
            }

            // settings
            ColumnLayout {
                visible: root.showSettings
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                LogosText { text: "IPFS gateways, tried in order (one per line). Every block is checked against its hash, so a gateway cannot change content; it can only fail or see which CIDs you ask for. A local node (http://127.0.0.1:8080) keeps even that private."; wrapMode: Text.WordWrap; Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 12 }
                ScrollView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 90
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
                            root.core("set_gateways", [JSON.stringify(list)], function (raw) {
                                var r = root.j(raw)
                                if (r && r.ok) { root.say("Gateways saved"); root.refresh() } else root.error = (r && r.error) || "Saving failed"
                            })
                        }
                    }
                    LogosButton {
                        text: "Restore defaults"
                        onClicked: root.core("set_gateways", ["[]"], function (raw) { var r = root.j(raw); if (r && r.ok) { gwField.text = r.gateways.join("\n"); root.refresh() } })
                    }
                }
                LogosText { text: "Your IPFS node (Kubo RPC API, then its gateway). When it answers it is read from first, and \"Keep a copy\" pins documents there so they outlive Fileverse's pinning."; wrapMode: Text.WordWrap; Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 12 }
                RowLayout {
                    Layout.fillWidth: true
                    Field { id: nodeApiField; Layout.fillWidth: true; placeholderText: "http://127.0.0.1:5001"; text: root.snap.node ? root.snap.node.api : "" }
                    Field { id: nodeGwField; Layout.fillWidth: true; placeholderText: "http://127.0.0.1:8080"; text: root.snap.node ? root.snap.node.gateway : "" }
                    LogosButton {
                        text: "Save node"
                        onClicked: root.core("set_ipfs_node", [nodeApiField.text, nodeGwField.text], function (raw) {
                            var r = root.j(raw)
                            if (r && r.ok) { root.say("IPFS node saved"); root.refresh(); root.refreshNode() } else root.error = (r && r.error) || "Saving failed"
                        })
                    }
                }
                LogosText {
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    font.pixelSize: 12
                    color: root.node.reachable ? Theme.palette.success : Theme.palette.textTertiary
                    text: root.node.reachable ? ("Connected: " + (root.node.agent || "IPFS node") + " " + root.shortId(root.node.peerId)) : "No IPFS node answering at " + (root.node.api || "http://127.0.0.1:5001") + ". Run one (e.g. Kubo) to keep copies."
                }
                LogosText { text: "Gnosis RPC: " + (root.snap.rpc || "eth_rpc_module") + ". Change the endpoint (or set a proxy/Tor) in the Ethereum RPC app; this viewer adds chain 100 there only if it is missing."; wrapMode: Text.WordWrap; Layout.fillWidth: true; textFormat: Text.PlainText; font.pixelSize: 12; color: Theme.palette.textTertiary }
            }

            LogosText {
                visible: root.error !== ""
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                textFormat: Text.PlainText
                color: Theme.palette.warning
                text: root.error.charAt(0).toUpperCase() + root.error.slice(1)
            }

            LogosText {
                visible: !root.docv && !root.loading && root.error === ""
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                textFormat: Text.PlainText
                color: Theme.palette.textTertiary
                text: "Open a share link to read a document. The key after #key= never leaves this device."
            }

            Item { visible: !root.docv; Layout.fillHeight: true }

            // document header
            ColumnLayout {
                visible: root.docv !== null
                Layout.fillWidth: true
                spacing: 2
                LogosText { Layout.fillWidth: true; wrapMode: Text.WordWrap; textFormat: Text.PlainText; font.pixelSize: 24; font.bold: true; text: root.docv ? (root.docv.title || "Untitled") : "" }
                RowLayout {
                    Layout.fillWidth: true
                    LogosText {
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        font.pixelSize: 12
                        color: Theme.palette.textTertiary
                        text: root.docv ? ((root.docv.kind === "dsheet" ? "dSheet" : "dDoc") + " · " + root.docv.layout + " portal " + root.shortId(root.docv.portal) + " · file " + root.docv.fileId
                                           + (root.docv.updatedAt ? " · updated " + new Date(root.docv.updatedAt).toLocaleString() : "")) : ""
                    }
                    LogosButton {
                        visible: root.node.reachable || root.kept
                        enabled: root.node.reachable && !root.keeping
                        text: root.keeping ? "Keeping…" : (root.kept ? "Kept ✓" : "Keep a copy")
                        onClicked: root.keepCopy()
                    }
                    LogosButton { visible: root.deck.length > 0; text: "Present"; onClicked: root.present() }
                    LogosButton { visible: root.docv && root.docv.markdown !== undefined; text: "Copy Markdown"; onClicked: root.copy(root.docv.markdown) }
                    LogosButton { text: root.showDetails ? "Hide sources" : "Sources"; onClicked: root.showDetails = !root.showDetails }
                }
            }

            // provenance
            ColumnLayout {
                visible: root.showDetails && root.docv !== null
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

            // tabs (docs) / sheet tabs
            Flow {
                Layout.fillWidth: true
                spacing: Theme.spacing.small
                visible: root.tabs.length > 1 || root.sheets.length > 1
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
                visible: root.tabs.length > 0
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 6
                model: root.blocks
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
                visible: root.sheet !== null
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
    readonly property var deck: root.tabs.length > 0 ? (root.tabs[Math.min(root.tabIndex, root.tabs.length - 1)].slides || []) : []
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
        width: toastText.implicitWidth + 32
        height: 36
        radius: 18
        color: Theme.palette.surfaceRaised
        LogosText { id: toastText; anchors.centerIn: parent; textFormat: Text.PlainText; text: root.toast }
    }
}
