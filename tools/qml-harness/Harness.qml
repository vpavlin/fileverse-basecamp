// Offscreen render check for fileverse_ui/Main.qml.
// A mock `logos` (an id in this context, which the loaded view's context inherits)
// answers the core calls from fixtures; MODE picks what the view shows; the window
// is grabbed to $OUT after it settles.
import QtQuick
import QtQuick.Window

Window {
    id: win
    width: 1100
    height: 760
    visible: true

    // Arguments after "--": MODE [SLIDE] OUT.png, MODE one of doc | slides | sheet | error.
    readonly property var args: Qt.application.arguments.slice(Qt.application.arguments.indexOf("--") + 1)
    property string mode: args[0]
    property int slideAt: mode === "slides" && args.length > 2 ? parseInt(args[1]) : 0
    property var docJson: null

    QtObject {
        id: logos
        signal moduleEventReceived(string moduleName, string eventName, var data)
        function onModuleEvent(m, e) {}
        function callModuleAsync(module, method, args, cb, timeout) {
            Qt.callLater(function () { cb(JSON.stringify(win.answer(method, args))) })
        }
    }

    function answer(method, args) {
        if (method === "snapshot") return { ok: true, version: "0.1.0", rpc: "eth_rpc_module (chain 100)", gateways: ["https://gateway.pinata.cloud", "https://ipfs.fileverse.io"],
            recent: [ { url: "x", portal: "0x81fb962e2088De6925AffA4E068dd3FAF3EFE163", fileId: 57, title: "Fileverse Agents SDK - Docs 💛", kind: "ddoc" },
                      { url: "y", portal: "0x1111111111111111111111111111111111111111", fileId: 7, title: "Budget <b>2026</b>", kind: "dsheet" } ] }
        if (method === "open") return mode === "error" ? { ok: true, jobId: "j1" } : { ok: true, jobId: "j1" }
        if (method === "job") {
            if (mode === "error") return { ok: true, state: "failed", error: "the link key does not open this file (wrong or truncated link?)" }
            return { ok: true, state: "done", elapsedMs: 900, result: mode === "sheet" ? win.sheetDoc() : win.docJson }
        }
        return { ok: true }
    }

    function sheetDoc() {
        var rows = [["Item", "Cost", "Note"]]
        for (var i = 1; i < 40; i++) rows.push(["Row " + i, "$" + (i * 1.5).toFixed(2), i % 3 === 0 ? "<img src=x>" : ""])
        return { ok: true, kind: "dsheet", title: "Budget 2026", layout: "v3", portal: "0x1111111111111111111111111111111111111111", fileId: 7,
                 provenance: { steps: ["read files(7)"], blobs: [] },
                 sheets: [ { name: "Budget", rows: rows, rowCount: 40, colCount: 3, truncated: false }, { name: "Notes", rows: [["a"]], rowCount: 1, colCount: 1, truncated: false } ] }
    }

    Loader {
        id: view
        anchors.fill: parent
        source: Qt.resolvedUrl("../../fileverse_ui/Main.qml")
        onStatusChanged: if (status === Loader.Error) { console.log("LOAD ERROR"); Qt.exit(2) }
        onLoaded: Qt.callLater(function () { item.showDetails = (win.mode === "doc"); item.open("https://docs.fileverse.io/0x81fb962e2088De6925AffA4E068dd3FAF3EFE163/57#key=abc") })
    }

    Component.onCompleted: {
        var xhr = new XMLHttpRequest()
        xhr.open("GET", Qt.resolvedUrl("doc.json"), false)
        xhr.send()
        win.docJson = JSON.parse(xhr.responseText)
    }

    Timer {
        interval: 1800; running: win.mode === "slides"
        onTriggered: { view.item.present(); for (var i = 0; i < win.slideAt; i++) view.item.nextSlide() }
    }
    Timer {
        interval: 3500; running: true
        onTriggered: {
            if (win.mode === "slides") console.log("deck", view.item.deck.length, "slides; showing", win.slideAt + 1, "kind", view.item.slide ? view.item.slide.kind : "-")
            if (win.mode === "error") console.log("error shown:", view.item.error)
            view.grabToImage(function (r) { r.saveToFile(win.args[win.args.length - 1]); console.log("SAVED"); Qt.exit(0) })
        }
    }
}
