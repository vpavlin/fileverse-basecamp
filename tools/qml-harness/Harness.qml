// Offscreen render check for fileverse_ui/Main.qml.
// A mock `logos` (an id in this context, which the loaded view's context inherits)
// answers the core calls from fixtures. Arguments after "--": MODE [N] OUT.png with
// MODE one of doc | slides | sheet | error | settings | portal | import | edit | editslides | wysiwyg | slidecols.
import QtQuick
import QtQuick.Window

Window {
    id: win
    width: 1200
    height: 800
    visible: true

    readonly property var args: Qt.application.arguments.slice(Qt.application.arguments.indexOf("--") + 1)
    property string mode: args[0]
    property int slideAt: args.length > 2 ? parseInt(args[1]) : 0
    property var docJson: null
    property var lastJob: ""

    QtObject {
        id: logos
        signal moduleEventReceived(string moduleName, string eventName, var data)
        function onModuleEvent(m, e) {}
        function callModule(module, method, args) { return JSON.stringify(win.answer(method, args)) }
        function callModuleAsync(module, method, args, cb, timeout) {
            Qt.callLater(function () { cb(JSON.stringify(win.answer(method, args))) })
        }
    }

    function draft() {
        var md = "# Logos plan\n\nWe go **self-sovereign**: [Logos](https://logos.co).\n\n## Steps\n\n- IPFS node module\n- Logos Storage\n  - copy blocks\n\n1. edit\n2. publish\n\n> no servers\n\n```\nfvbc://ipfs/bafk…\n```\n\n| what | where |\n|---|---|\n| blocks | IPFS + Storage |\n"
        return { id: "abc123", title: "Logos plan", markdown: md, sourceUrl: "", preview: win.previewOf(), published: mode === "edit" ? { link: "fvbc://ipfs/bafkreiexample#key=k", at: 1792700000000 } : null }
    }
    function previewOf() {
        var b = [
            { t: "h1", html: "Logos plan", text: "Logos plan" },
            { t: "p", html: "We go <b>self-sovereign</b>: <a href=\"https://logos.co\">Logos</a>.", text: "We go self-sovereign: Logos." },
            { t: "h2", html: "Steps", text: "Steps" },
            { t: "li", html: "IPFS node module", text: "IPFS node module" },
            { t: "li", html: "Logos Storage", text: "Logos Storage" },
            { t: "li", depth: 1, html: "copy blocks", text: "copy blocks" },
            { t: "oli", num: 1, html: "edit", text: "edit" },
            { t: "oli", num: 2, html: "publish", text: "publish" },
            { t: "p", quote: true, html: "no servers", text: "no servers" },
            { t: "code", html: "", text: "fvbc://ipfs/bafk…" },
            { t: "table", html: "", text: "", rows: [["what", "where"], ["blocks", "IPFS + Storage"]] }
        ]
        return [{ id: "default", name: "Tab 1", depth: 0, blocks: b, slides: [{ kind: "title", blocks: [0] }, { kind: "content", blocks: [1] }, { kind: "content", blocks: [2, 3, 4, 5, 6, 7, 8] }, { kind: "content", blocks: [9, 10] }] }]
    }

    function answer(method, args) {
        if (method === "snapshot") return { ok: true, version: "0.3.0", rpc: "eth_rpc_module (chain 100)", gateways: ["https://gateway.pinata.cloud"], storedBlocks: 6,
            node: { api: "", gateway: "" },
            recent: [ { url: "x", portal: "0x81fb962e2088De6925AffA4E068dd3FAF3EFE163", fileId: 57, title: "Fileverse Agents SDK - Docs 💛", kind: "ddoc", kept: ["a"], inStorage: ["a"] },
                      { url: "fvbc://ipfs/b#key=k", layout: "logos", title: "Logos plan", kind: "ddoc" },
                      { url: "fvown://0x2222222222222222222222222222222222222222/1", portal: "0x2222222222222222222222222222222222222222", fileId: 1, title: "Budget <b>2026</b>", kind: "dsheet" } ],
            portals: [ { portal: "0x2222222222222222222222222222222222222222", kind: "ec", source: "docs.fileverse.io", hasPq: false }, { portal: "0x3333333333333333333333333333333333333333", kind: "legacy", source: "docs.fileverse.io" } ],
            drafts: [ { id: "abc123", title: "Logos plan", updatedAt: 1792700000000, link: "fvbc://x", publishedAt: 1792700000000 }, { id: "d2", title: "Notes", updatedAt: 1792690000000 } ] }
        if (method === "node_status") return { ok: true, reachable: true, source: "ipfs_module", peers: 43, pins: 12, repoSize: 52428800, api: "http://127.0.0.1:40263", gateway: "http://127.0.0.1:34231", peerId: "12D3KooWHeYVZC3hgJsXByFqD2tnVPdbhEfHt8rzJbXh5" }
        if (method === "storage_status") return { ok: true, available: true, peerId: "16Uiu2…" }
        if (method === "draft_get" || method === "draft_save" || method === "draft_save_rich" || method === "draft_create") return { ok: true, draft: win.draft() }
        // A crude stand-in for the core's Markdown -> HTML (the real one is tested in Rust).
        if (method === "rich_html") return { ok: true, html: String(args[0]).split("\n\n").map(function (b) {
            var m = b.match(/^(#{1,3}) (.*)/)
            if (m) return "<h" + m[1].length + ">" + m[2] + "</h" + m[1].length + ">"
            if (/^[-\d]/.test(b)) return "<ul>" + b.split("\n").map(function (l) { return "<li>" + l.replace(/^\s*(-|\d+\.) (\[[ x]\] )?/, "") + "</li>" }).join("") + "</ul>"
            if (/^> /.test(b)) return "<blockquote><p>" + b.substring(2) + "</p></blockquote>"
            return "<p>" + b.replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>") + "</p>"
        }).join("") }
        if (method === "rich_kind") return { ok: true, kind: "h1" }
        if (method === "image") return { ok: true, jobId: "jimg" }
        if (method === "diagram") return { ok: true, jobId: args[0].indexOf("BROKEN") >= 0 ? "jdiagbad" : "jdiag" }
        if (method === "job") {
            var id = args[0]
            if (id === "jimg") return { ok: true, state: "done", result: { ok: true, dataUri: win.samplePng } }
            // an SVG as the core draws it (a flowchart, Logos dark colours)
            if (id === "jdiag") return { ok: true, state: "done", result: { ok: true, fileUrl: String(Qt.resolvedUrl("diagram.svg")), width: 736, height: 245 } }
            if (id === "jdiagbad") return { ok: true, state: "failed", error: "not a diagram this renderer knows: parse error" }
            if (mode === "error") return { ok: true, state: "failed", error: "the link key does not open this file (wrong or truncated link?)" }
            if (id === "jlist") return { ok: true, state: "done", result: { ok: true, portal: "0x2222222222222222222222222222222222222222", files: [
                { fileId: 3, title: "Meeting notes", kind: "ddoc", updatedAt: 1792700000000, url: "fvown://0x2222222222222222222222222222222222222222/3" },
                { fileId: 2, kind: "ddoc", error: "locked with Fileverse's post-quantum scheme, whose library is not public; open it with a share link instead", url: "u" },
                { fileId: 1, title: "Budget 2026", kind: "dsheet", updatedAt: 1792600000000, url: "u1" },
                { fileId: 0, title: "First doc", kind: "ddoc", url: "u0" } ] } }
            return { ok: true, state: "done", elapsedMs: 900, result: mode === "sheet" ? win.sheetDoc() : win.docJson }
        }
        if (method === "list_portal") return { ok: true, jobId: "jlist" }
        if (method === "portal_files") return { ok: true, portal: "0x2222222222222222222222222222222222222222", listedAt: Date.now(), backup: { "3": { local: true, ipfs: true, storage: true }, "1": { local: true, ipfs: true, storage: false } },
            files: [ { fileId: 3, title: "Meeting notes", kind: "ddoc", updatedAt: 1792700000000, url: "u3", cids: ["a","b",""] }, { fileId: 2, kind: "ddoc", error: "locked with Fileverse's post-quantum scheme, whose library is not public; open it with a share link instead", url: "u" },
                     { fileId: 1, title: "Budget 2026", kind: "dsheet", updatedAt: 1792600000000, url: "u1" }, { fileId: 0, title: "First doc", kind: "ddoc", url: "u0" } ] }
        if (method === "open") return { ok: true, jobId: "j1" }
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
        onLoaded: Qt.callLater(function () {
            var it = item
            it.showDetails = (win.mode === "doc")
            it.showSettings = (win.mode === "settings")
            if (win.mode === "portal") { it.section = "files"; it.showPortal("0x2222222222222222222222222222222222222222") }
            else if (win.mode === "import") { it.section = "files"; it.mode = "import" }
            else if (win.mode === "edit" || win.mode === "editslides" || win.mode === "wysiwyg") { it.section = "drafts"; it.openDraft("abc123") }
            else it.open("https://docs.fileverse.io/0x81fb962e2088De6925AffA4E068dd3FAF3EFE163/57#key=abc")
        })
    }

    // a landscape picture standing in for a decrypted image
    readonly property string samplePng: "data:image/svg+xml;utf8," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="640" height="400"><rect width="640" height="400" fill="#2d6cdf"/><circle cx="470" cy="140" r="70" fill="#ffd34d"/><path d="M0 400 L220 180 L380 330 L470 250 L640 400 Z" fill="#1e8f5a"/></svg>')
    Component.onCompleted: {
        var xhr = new XMLHttpRequest()
        xhr.open("GET", Qt.resolvedUrl("doc.json"), false)
        xhr.send()
        win.docJson = JSON.parse(xhr.responseText)
        if (mode === "diagram" || mode === "diagramonly") {
            // an H2 with a diagram under it, then a diagram alone, then one that cannot be drawn
            var dt = win.docJson.tabs[0], m = dt.blocks.length
            dt.blocks.push({ t: "h2", html: "How the modules fit", text: "How the modules fit" })
            dt.blocks.push({ t: "code", lang: "mermaid", html: "", text: "flowchart LR\n  ui --> core" })
            dt.blocks.push({ t: "code", lang: "mermaid", html: "", text: "flowchart LR\n  ui --> core %% alone" })
            dt.blocks.push({ t: "code", lang: "mermaid", html: "", text: "BROKEN diagram" })
            dt.slides.push({ kind: "content", blocks: [m, m + 1] })
            dt.slides.push({ kind: "image", blocks: [m + 2] })
            dt.slides.push({ kind: "content", blocks: [m + 3] })
        }
        if (mode === "slidecols") {
            // dDocs' image beside text: an H2, then columns (text | image), align-right
            var t = win.docJson.tabs[0], n = t.blocks.length
            t.blocks.push({ t: "h2", html: "Image beside text", text: "Image beside text" })
            t.blocks.push({ t: "columns", layout: "align-right", text: "", cols: [
                [ { t: "p", html: "A column of text sits to the left of the picture, the way dDocs lays out a <b>two-column</b> block.", text: "x" },
                  { t: "li", html: "Columns are never split across slides", text: "x" },
                  { t: "li", html: "Images are decrypted locally", text: "x" } ],
                [ { t: "image", text: "", img: { cid: "bafkreitest", key: "k", nonce: "n", tag: "t", mime: "image/png" } } ] ] })
            t.slides.push({ kind: "content", blocks: [n, n + 1] })
        }
    }

    Timer {
        interval: 1800; running: win.mode === "slides" || win.mode === "editslides" || win.mode === "slidecols" || win.mode === "diagram" || win.mode === "diagramonly"
        onTriggered: {
            view.item.present()
            var to = win.mode === "slidecols" ? view.item.deck.length - 1
                   : win.mode === "diagram" ? view.item.deck.length - 3
                   : win.mode === "diagramonly" ? view.item.deck.length - 2 + win.slideAt : win.slideAt
            for (var i = 0; i < to; i++) view.item.nextSlide()
            if (win.mode === "slidecols") view.item.imageSrc({ cid: "bafkreitest" })
        }
    }
    Timer {
        interval: 3500; running: true
        onTriggered: {
            if (win.mode === "slides" || win.mode === "editslides" || win.mode === "slidecols" || win.mode === "diagram" || win.mode === "diagramonly") {
                console.log("deck", view.item.deck.length, "slides; showing", view.item.slideIndex + 1, "kind", view.item.slide ? view.item.slide.kind : "-")
                console.log("slides:", JSON.stringify(view.item.deck.map(function (d) { return d.blocks })))
            }
            if (win.mode === "error") console.log("error shown:", view.item.error)
            if (win.mode === "edit") console.log("editor md chars:", view.item.draft ? view.item.draft.markdown.length : -1, "preview blocks:", view.item.blocks.length)
            view.grabToImage(function (r) { r.saveToFile(win.args[win.args.length - 1]); console.log("SAVED"); Qt.exit(0) })
        }
    }
}
