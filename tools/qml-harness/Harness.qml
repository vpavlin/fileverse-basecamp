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
    // MODE or MODE-light / MODE-dark: the paper colours to show
    property string mode: String(args[0]).replace(/-(light|dark)$/, "").replace("chatstream-end", "chatstream")
    property string paperArg: (String(args[0]).match(/-(light|dark)$/) || [null, "basecamp"])[1]
    property int slideAt: args.length > 2 ? parseInt(args[1]) : 0
    property var docJson: null
    property var lastJob: ""
    property var chatRequests: []
    property var turns: []
    property bool lastStructured: false
    property var prefs: ({})
    property int polls: 0
    readonly property string chatReply: "Two small changes would make it clearer:\n\n```edit\n<<<<<<< FIND\n- Logos Storage\n=======\n- Logos Storage (backups)\n>>>>>>> REPLACE\n```\n\n```edit\n<<<<<<< FIND\nWe go **self-sovereign**\n=======\nWe go fully **self-sovereign**\n>>>>>>> REPLACE\n```"

    QtObject {
        id: logos
        signal moduleEventReceived(string moduleName, string eventName, var data)
        function onModuleEvent(m, e) {}
        function callModule(module, method, args) { return JSON.stringify(win.answer(method, args)) }
        function callModuleAsync(module, method, args, cb, timeout) {
            Qt.callLater(function () { cb(JSON.stringify(win.answer(method, args))) })
        }
        // Basecamp's intents (AI Gateway's ai.ask): "aiunavail" = not installed
        function request(intent, params, cb) {
            console.log("REQUEST", intent, JSON.stringify(params).substring(0, 400))
            if (win.mode === "aiunavail") { Qt.callLater(function () { cb({ ok: false, error: "unavailable" }) }); return }
            if (intent === "ai.session") {
                console.log("SESSION title=" + params.title + " document=" + String(params.document).length + " system=" + String(params.system).length)
                if (win.mode === "chatstateless") { Qt.callLater(function () { cb({ ok: false, error: "unavailable" }) }); return }
                Qt.callLater(function () { cb({ ok: true, data: Object.assign({ sessionId: "s1", model: "llama-3.3-70b", connector: "venice", privacy: "no-retention" },
                    win.mode === "chatprovider" ? { provider: "other_ai_core", api: "ai-session/1" } : win.mode === "chatapi" ? { provider: "other_ai_core", api: "ai-session/2" } : {}) }) })
                return
            }
            if (intent === "ai.chat") {
                win.chatRequests.push(params.messages)
                var reply = "Two small changes would make it clearer:\n\n```edit\n<<<<<<< FIND\n- Logos Storage\n=======\n- Logos Storage (backups)\n>>>>>>> REPLACE\n```\n\n```edit\n<<<<<<< FIND\nWe go **self-sovereign**\n=======\nWe go fully **self-sovereign**\n>>>>>>> REPLACE\n```"
                Qt.callLater(function () { cb({ ok: true, data: { text: reply, model: "llama-3.3-70b", connector: "venice", privacy: "no-retention", usage: {} } }) })
                return
            }
            Qt.callLater(function () { cb({ ok: true, data: { text: "```markdown\nLogos plan, **without servers**\n```", model: "llama-3.3-70b", connector: "venice", privacy: "no-retention", usage: {} } }) })
        }
    }

    function draft() {
        var md = "# Logos plan\n\nWe go **self-sovereign**: [Logos](https://logos.co).\n\n## Steps\n\n- IPFS node module\n- Logos Storage\n  - copy blocks\n\n1. edit\n2. publish\n\n> no servers\n\n```\nfvbc://ipfs/bafk…\n```\n\n| what | where |\n|---|---|\n| blocks | IPFS + Storage |\n"
        return { id: "abc123", title: "Logos plan", markdown: md, sourceUrl: "", preview: win.previewOf(), published: mode === "edit" ? { link: "fvbc://ipfs/bafkreiexample#key=k", at: 1792700000000 } : null,
                 reflection: (mode === "shared" || mode === "sharedmd" || mode === "sharedcatchup") ? { id: "ab".repeat(32) } : undefined }
    }
    function previewOf() {
        var b = [
            { t: "h1", html: "Logos plan", text: "Logos plan" },
            { t: "p", html: "We go <b>self-sovereign</b>: <a href=\"https://logos.co\">Logos</a>. Drafts live in <code>~/.local/share/Logos/drafts</code>.", text: "We go self-sovereign: Logos." },
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
        if (method === "chat_load") return { ok: true, chat: null }
        if (method === "ai_call") {
            var pl = JSON.parse(args[1])
            var aa = Array.isArray(pl) ? pl : pl.args
            console.log("AICALL", args[0], "provider=" + (Array.isArray(pl) ? "(bare array)" : pl.provider))
            if (args[0] === "sessionChat") {
                var t = JSON.parse(aa[1]); win.turns.push(t)
                // as AI Gateway: a turn needs a message (or messages ending in a non-empty user message)
                var lastU = t.messages ? t.messages[t.messages.length - 1] : null
                if (!(t.message && String(t.message).trim()) && !(lastU && lastU.role === "user" && String(lastU.content).trim())) {
                    console.log("BAD TURN: no message", JSON.stringify(t).substring(0, 200))
                    return { ok: false, code: "bad_request", error: "the turn needs a message" }
                }
                console.log("TURN", aa[0], JSON.stringify(Object.keys(t)), t.messages ? t.messages.length + " messages" : "", t.document ? "document " + t.document.length : "no document", "| context:", JSON.stringify(String(t.context || "").substring(0, 60)), "| schema:", !!t.responseFormat, "| reset:", !!t.resetHistory)
                // mode chatprose: the provider refuses the schema (the job errors)
                win.lastStructured = !!t.responseFormat
                return { ok: true, jobId: "job" + win.turns.length }
            }
            // mode chatdelta: AI Gateway 0.1.10's jobDelta, the answer in 40 pieces; other modes: an older core refuses it
            if (args[0] === "jobDelta") {
                if (win.mode !== "chatdelta") return { ok: false, error: "not an AI Gateway call Fileverse makes: jobDelta" }
                var fullD = JSON.stringify({ reply: "Streamed by delta: ünïcödé ✓ \"quoted\" line.", edits: [{ find: "- Logos Storage", replace: "- Logos Storage (backups)", why: "Says what Storage is for." }] })
                win.polls++
                var since = parseInt(aa[1]); var upto = Math.min(fullD.length, Math.round(fullD.length * win.polls / 40))
                // positions in JS chars here stand in for the byte counts: the view only hands them back
                if (win.polls > 1) console.log("DPARTIAL", win.polls - 1, JSON.stringify(view.item.chatPartial).substring(0, 120))
                if (upto >= fullD.length) return { ok: true, state: "done", done: true, offset: since, delta: fullD.substring(since), length: fullD.length, model: "m" }
                return { ok: true, state: "running", done: false, offset: since, delta: fullD.substring(since, upto), length: upto }
            }
            if (args[0] === "job") {
                if (win.mode === "chatdelta") { console.log("JOB called after delta, polls=" + win.polls); return { state: "done", text: JSON.stringify({ reply: "Streamed by delta: ünïcödé ✓ \"quoted\" line.", edits: [{ find: "- Logos Storage", replace: "- Logos Storage (backups)", why: "Says what Storage is for." }] }), model: "llama-3.3-70b", connector: "venice", privacy: "no-retention" } }
                if (win.mode === "chatstream") {
                    // the answer arrives in 40 pieces, one per poll (~14 s): the screenshot is mid-stream
                    var full = JSON.stringify({ reply: "Here is a longer answer, so you can watch it arrive.\n\n- First, the \"title\" could say what the plan is for.\n- Second, the steps could be numbered.\n- Third, ünïcödé ✓ and a tab\there.\n\nTwo edits follow.", edits: [
                        { find: "- Logos Storage", replace: "- Logos Storage (backups)", why: "Says what Storage is for." },
                        { find: "We go **self-sovereign**", replace: "We go fully **self-sovereign**", why: "Stronger opening." } ] })
                    win.polls++
                    var it = view.item
                    if (win.polls > 1) console.log("PARTIAL", win.polls - 1, JSON.stringify(it.chatPartial).substring(0, 200))
                    if (win.polls < 40) return { state: "running", text: full.substring(0, Math.round(full.length * win.polls / 40)) }
                    return { state: "done", text: full, model: "llama-3.3-70b", connector: "venice", privacy: "no-retention" }
                }
                if (win.lastStructured && win.mode === "chatprose") return { state: "error", error: "provider: response_format not supported" }
                if (win.lastStructured && win.mode === "chatbadjson") return { state: "done", text: "Sure! Here you go: " + win.chatReply, model: "m" }
                if (win.lastStructured) return { state: "done", text: JSON.stringify({ reply: "Two small changes would make it clearer:", edits: [
                    { find: "- Logos Storage", replace: "- Logos Storage (backups)", why: "Says what Storage is for." },
                    { find: "We go **self-sovereign**", replace: "We go fully **self-sovereign**", why: "Stronger opening." } ] }), model: "llama-3.3-70b", connector: "venice", privacy: "no-retention" }
                return { state: "done", text: win.chatReply, model: "llama-3.3-70b", connector: "venice" }
            }
            if (args[0] === "closeSession") { console.log("CLOSE", aa[0]); return { ok: true } }
            return { ok: false, error: "?" }
        }
        if (method === "chat_save") return { ok: true }
        if (method === "set_pref") { var np = JSON.parse(JSON.stringify(win.prefs)); np[args[0]] = JSON.parse(args[1]); win.prefs = np; console.log("SET_PREF", args[0], args[1]); return { ok: true, prefs: Object.assign({ paper: win.paperArg }, win.prefs) } }
        if (method === "snapshot") return { ok: true, prefs: Object.assign({ paper: win.paperArg }, win.prefs), version: "0.3.0", rpc: "eth_rpc_module (chain 100)", gateways: ["https://gateway.pinata.cloud"], storedBlocks: 6,
            node: { api: "", gateway: "" },
            recent: [ { url: "x", portal: "0x81fb962e2088De6925AffA4E068dd3FAF3EFE163", fileId: 57, title: "Fileverse Agents SDK - Docs 💛", kind: "ddoc", kept: ["a"], inStorage: ["a"] },
                      { url: "fvbc://ipfs/b#key=k", layout: "logos", title: "Logos plan", kind: "ddoc" },
                      { url: "fvown://0x2222222222222222222222222222222222222222/1", portal: "0x2222222222222222222222222222222222222222", fileId: 1, title: "Budget <b>2026</b>", kind: "dsheet" } ],
            portals: [ { portal: "0x2222222222222222222222222222222222222222", kind: "ec", source: "docs.fileverse.io", hasPq: false }, { portal: "0x3333333333333333333333333333333333333333", kind: "legacy", source: "docs.fileverse.io" } ],
            drafts: [ { id: "abc123", title: "Logos plan", updatedAt: 1792700000000, link: "fvbc://x", publishedAt: 1792700000000 }, { id: "d2", title: "Notes", updatedAt: 1792690000000, shared: true, reflection: "ab" } ] }
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
        // reflection_core, as fileverse_core passes it through (reflection_call)
        if (method === "reflection_call") {
            var rm = args[0], ra = JSON.parse(args[1] || "[]")
            var cur = { id: "ab".repeat(32), name: "Logos plan", text: win.draft().markdown, version: 3, online: ["b0b", "a11"],
                        cursors: [ { author: "b0b", pos: 12, anchor: 12 }, { author: "a11", pos: 60, anchor: 60 } ],
                        delivery: { unpublished: false, catchingUp: false, loroPending: mode === "sharedcatchup", waiting: 0, peersMissingOurs: [] } }
            var st = { ok: true, me: "me", transport: { status: "Logos: Connected" }, syncMode: "logos",
                       settings: { shareCursor: true, networks: mode === "sharedold" ? undefined : "both" },
                       docs: [ { id: "ab".repeat(32), name: "Logos plan", online: 2 } ] }
            if (rm === "state") { if (ra[0]) st.current = cur; return st }
            if (rm === "set_text") { cur.version = 4; cur.editVersion = 4; return { ok: true, current: cur } }
            if (rm === "set_settings") return { ok: true, settings: st.settings }
            return { ok: true }
        }
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
            it.showSettings = (win.mode === "settings" || win.mode === "aiprompts" || win.mode === "aipromptedit")
            if (win.mode === "aiprompts" || win.mode === "aipromptedit") promptTimer.start()
            if (win.mode === "portal") { it.section = "files"; it.showPortal("0x2222222222222222222222222222222222222222") }
            else if (win.mode === "localfiles") { it.section = "files"; it.showPortal("") }
            else if (win.mode === "import") { it.section = "files"; it.mode = "import" }
            else if (win.mode === "confirmdel") { it.section = "drafts"; it.openDraft("abc123"); Qt.callLater(function () { it.confirm("Delete “Logos plan”?", "It is deleted from this device, with its AI chat. This cannot be undone. The published copy stays where it was published.", "Delete", function () { console.log("DELETED") }) }) }
            else if (win.mode === "edit" || win.mode === "editslides" || win.mode === "wysiwyg" || win.mode === "shared" || win.mode === "sharedcatchup" || win.mode === "sharedmd" || win.mode === "editmd" || win.mode === "editai" || win.mode === "editaiapply" || win.mode === "editmdai" || win.mode === "aiunavail" || win.mode === "chat" || win.mode === "chatmd" || win.mode === "chatapply" || win.mode === "chatstateless" || win.mode === "chatprose" || win.mode === "chatbadjson" || win.mode === "chatreset" || win.mode === "chatstream" || win.mode === "chatdelta" || win.mode === "chatprovider" || win.mode === "chatapi") {
                it.section = "drafts"; it.openDraft("abc123")
                if (win.mode === "sharedmd" || win.mode === "editmd" || win.mode === "editmdai" || win.mode === "chatmd") Qt.callLater(function () { it.setVisualEdit(false) })
                if (win.mode === "editai" || win.mode === "editaiapply" || win.mode === "editmdai" || win.mode === "aiunavail") aiTimer.start()
                if (win.mode === "chat" || win.mode === "chatmd" || win.mode === "chatapply" || win.mode === "chatstateless" || win.mode === "chatprose" || win.mode === "chatbadjson" || win.mode === "chatreset" || win.mode === "chatstream" || win.mode === "chatdelta" || win.mode === "chatprovider" || win.mode === "chatapi") chatTimer.start()
            }
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
    // AI chat: open the panel, send a message; then apply the edits and send another; then report
    Timer {
        id: chatTimer; interval: 1500
        onTriggered: { view.item.chatToggle(); view.item.chatSend("Can you make this clearer?"); chatTimer2.start() }
    }
    Timer {
        id: chatTimer2; interval: 1800
        onTriggered: {
            var it = view.item
            if (win.mode === "chatreset") { it.chatClear(); it.chatSend("Start over, please."); chatTimer3.start(); return }
            if (win.mode !== "chat" && win.mode !== "chatstream" && win.mode !== "chatdelta") {
                it.chatApplyAll(it.chatMsgs.length - 1)
                console.log("AFTER APPLY:", JSON.stringify(it.editorMarkdown().substring(0, 160)))
                console.log("EDIT STATES:", JSON.stringify(it.chatMsgs[it.chatMsgs.length - 1].edits.map(function (e) { return e.state })))
                it.chatSend("Thanks. And now?")
            }
            chatTimer3.start()
        }
    }
    Timer {
        id: chatTimer3; interval: 1500
        onTriggered: {
            win.chatRequests.forEach(function (msgs, n) {
                console.log("REQUEST " + (n + 1) + ": " + msgs.length + " messages; " + msgs.map(function (m) { return m.role + "(" + m.content.length + "):" + JSON.stringify(m.content.substring(0, 60)) }).join(" | "))
            })
            console.log("MSGS:", JSON.stringify(view.item.chatMsgs.map(function (m) { return m.role })))
            if (win.mode === "chatmd") { view.item.chatToggle() }
            console.log("SESSION STATE:", JSON.stringify(view.item.chatSession))
        }
    }
    Timer { id: aiPrint; interval: 400; onTriggered: console.log("AFTER REPLACE:", JSON.stringify(view.item.editorMarkdown().substring(0, 120))) }
    // AI prompts: change some, add an action, check what would be sent, reset one
    Timer {
        id: promptTimer; interval: 1200
        onTriggered: {
            var it = view.item
            var shorter = it.aiActions[2]
            it.aiSetPrompt("a:Shorter", shorter[1], "Make it tiny.")
            it.aiSetPrompt("system", it.aiSystem, "Answer like a pirate. Text only.")
            it.aiSetPrompt("translate", it.aiTranslateDefault, "Into {language}, please, word for word.")
            it.aiSetMine(-1, "Friendlier", "Make this friendlier.", "replace")
            it.aiSetMine(-1, "Shorter", "dup", "replace")
            it.aiSetPrompt("chat", it.chatRole, "You are a terse editor.")
            Qt.callLater(function () {
                console.log("ACTIONS", JSON.stringify(it.aiActionsNow))
                console.log("SYSTEM", it.aiSystemNow)
                console.log("TRANSLATE", it.aiTranslateNow)
                console.log("CHATSYS", it.chatSessionSystem.substring(0, 120))
                console.log("LEGACY", it.chatSystem.substring(0, 120))
                console.log("EDITED shorter=" + it.aiPromptEdited("a:Shorter", shorter[1]) + " clearer=" + it.aiPromptEdited("a:Clearer", it.aiActions[1][1]))
                it.aiSetPrompt("a:Shorter", shorter[1], shorter[1])
                Qt.callLater(function () {
                    console.log("AFTER RESET", JSON.stringify(win.prefs.aiPrompts))
                    if (win.mode === "aipromptedit") it.promptEdit = { key: "chat", title: "AI chat: who the AI is and how it talks", def: it.chatRole, text: it.aiPromptNow("chat", it.chatRole) }
                })
            })
        }
    }
    // AI: open the row, "Clearer" on the paragraph the cursor is in (the title), then maybe Replace
    Timer {
        id: aiTimer; interval: 1500
        onTriggered: {
            var it = view.item
            it.aiOpen = true
            it.aiRun("Clearer", "Rewrite this text to be clearer.", "target", "replace")
            if (win.mode === "editaiapply" || win.mode === "editmdai") Qt.callLater(function () { Qt.callLater(function () {
                it.aiApply("replace")
                aiPrint.start()
            }) })
            if (win.mode === "aiunavail") Qt.callLater(function () { Qt.callLater(function () { console.log("note:", it.aiNote, "| toast:", it.toast, "| error:", it.error) }) })
        }
    }
    Timer {
        interval: String(win.args[0]) === "chatstream-end" || win.mode === "chatdelta" ? 17000 : win.mode.indexOf("chat") === 0 ? 6500 : 3500; running: true
        onTriggered: {
            if (String(win.args[0]) === "chatstream-end" || win.mode === "chatdelta") { var lm = view.item.chatMsgs[view.item.chatMsgs.length - 1]; console.log("FINAL", lm.role, JSON.stringify(lm.text), "edits:", JSON.stringify((lm.edits || []).map(function (e) { return e.why })), "partial:", JSON.stringify(view.item.chatPartial), "busy:", view.item.chatBusy) }
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
