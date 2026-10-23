//! Your node is asked first but only briefly, and what is found elsewhere is put
//! into it: reading never waits on the node's network luck, and the node fills up
//! as a cache of everything read.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fileverse_core::ipfs::{raw_cid, Gateways};

/// A tiny HTTP server: `answer(request_head, body) -> (status, body)`.
fn serve(answer: impl Fn(&str, &[u8]) -> Option<(u16, Vec<u8>)> + Send + 'static) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut s = s;
            let mut buf = vec![0u8; 65536];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            match answer(&req, &buf[..n]) {
                Some((code, body)) => {
                    let _ = write!(s, "HTTP/1.1 {code} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                    let _ = s.write_all(&body);
                }
                // a node still searching the network: never answers
                None => std::thread::sleep(Duration::from_secs(30)),
            }
        }
    });
    addr
}

#[test]
fn a_slow_node_costs_seconds_and_then_gets_the_block() {
    let block = b"hello from a gateway".to_vec();
    let cid = raw_cid(&block).to_v1();
    let b2 = block.clone();
    let gateway = serve(move |_, _| Some((200, b2.clone())));
    let slow_node = serve(|_, _| None);
    let puts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let p2 = puts.clone();
    let api = serve(move |req, _| {
        p2.lock().unwrap().push(req.lines().next().unwrap_or("").to_string());
        Some((200, br#"{"Key":"x"}"#.to_vec()))
    });
    let mut gw = Gateways::new(vec![gateway], None);
    gw.node = Some((slow_node, api));
    let t0 = Instant::now();
    let (bytes, prov) = gw.get(&cid).unwrap();
    let took = t0.elapsed();
    assert_eq!(bytes, block);
    assert!(prov.verified);
    assert!(took < Duration::from_secs(8), "waited {took:?} on a node that never answers");
    let puts = puts.lock().unwrap();
    assert_eq!(puts.len(), 1, "the block was not added to the node");
    assert!(puts[0].starts_with("POST /api/v0/block/put?cid-codec=raw"), "{}", puts[0]);
}

#[test]
fn a_node_that_has_it_answers_first() {
    let block = b"already on my node".to_vec();
    let cid = raw_cid(&block).to_v1();
    let b2 = block.clone();
    let node = serve(move |_, _| Some((200, b2.clone())));
    let api = serve(|_, _| panic!("nothing should be put back"));
    let mut gw = Gateways::new(vec!["http://127.0.0.1:1".into()], None);
    gw.node = Some((node, api));
    let (bytes, prov) = gw.get(&cid).unwrap();
    assert_eq!(bytes, block);
    assert!(prov.gateway.ends_with("(your node)"), "{}", prov.gateway);
}
