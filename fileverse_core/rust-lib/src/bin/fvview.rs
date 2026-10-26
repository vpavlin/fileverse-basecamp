//! fvview: open a Fileverse share link from the command line, with the same
//! engine the Basecamp module uses.
//!
//!   fvview [--rpc URL] [--gateway URL]... [--portal ADDR]... [--keep IPFS_API] [--json] <share link>
//!
//! `--portal` names a portal to look in for a `/document/<id>` link.
//! `--keep` copies the document's verified blocks into that IPFS node and pins them.

use fileverse_core::{chain::HttpRpc, ipfs, node, viewer};

fn main() {
    let mut rpc = "https://rpc.gnosischain.com".to_string();
    let mut gws: Vec<String> = vec![];
    let mut json = false;
    let mut keep: Option<String> = None;
    let mut portals: Vec<String> = vec![];
    let mut link = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rpc" => rpc = args.next().expect("--rpc URL"),
            "--gateway" => gws.push(args.next().expect("--gateway URL")),
            "--json" => json = true,
            "--keep" => keep = Some(args.next().expect("--keep IPFS_API_URL")),
            "--portal" => portals.push(args.next().expect("--portal ADDRESS")),
            _ => link = Some(a),
        }
    }
    let Some(link) = link else {
        eprintln!("usage: fvview [--rpc URL] [--gateway URL]... [--json] <share link>");
        std::process::exit(2);
    };
    if gws.is_empty() {
        gws = ipfs::DEFAULT_GATEWAYS.iter().map(|s| s.to_string()).collect();
    }
    let cache = std::env::temp_dir().join("fvview-blocks");
    let gw = ipfs::Gateways::new(gws, Some(cache));
    let opened = viewer::open_link_in(&HttpRpc::new(&rpc), &gw, &link, &portals);
    if let (Some(api), Ok(v)) = (&keep, &opened) {
        let blobs = v["provenance"]["blobs"].as_array().unwrap();
        let roots: Vec<String> = blobs.iter().map(|b| b["cid"].as_str().unwrap().to_string()).collect();
        let blocks: Vec<String> = blobs.iter().flat_map(|b| b["blocks"].as_array().cloned().unwrap_or_default()).map(|c| c.as_str().unwrap().to_string()).collect();
        match node::Node::new(api, std::time::Duration::from_secs(30)).keep(&gw, &roots, &blocks) {
            Ok(n) => eprintln!("  - kept {n} blocks, pinned {} roots on {api}", roots.len()),
            Err(e) => {
                eprintln!("error: keep: {e}");
                std::process::exit(1);
            }
        }
    }
    match opened {
        Ok(v) if json => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
        Ok(v) => {
            for s in v["provenance"]["steps"].as_array().unwrap() {
                eprintln!("  - {}", s.as_str().unwrap());
            }
            for b in v["provenance"]["blobs"].as_array().unwrap() {
                eprintln!("  - {} from {} ({} bytes, verified: {})", b["cid"], b["gateway"], b["bytes"], b["verified"]);
            }
            if let Some(md) = v["markdown"].as_str() {
                println!("{md}");
            } else {
                for s in v["sheets"].as_array().unwrap() {
                    println!("## {}", s["name"].as_str().unwrap_or(""));
                    for r in s["rows"].as_array().unwrap() {
                        let cells: Vec<&str> = r.as_array().unwrap().iter().map(|c| c.as_str().unwrap_or("")).collect();
                        println!("| {} |", cells.join(" | "));
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
