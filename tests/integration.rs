use secp256k1::{rand, Keypair, Message, Secp256k1, XOnlyPublicKey};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tungstenite::Message as WsMsg;

struct Signer {
    keypair: Keypair,
    pubkey_hex: String,
}

impl Signer {
    fn generate() -> Self {
        let secp = Secp256k1::new();
        let keypair = Keypair::new(&secp, &mut rand::thread_rng());
        let (pubkey, _) = XOnlyPublicKey::from_keypair(&keypair);
        let pubkey_hex = hex::encode(pubkey.serialize());
        Signer {
            keypair,
            pubkey_hex,
        }
    }

    fn sign_event(&self, created_at: u64, kind: u64, tags: &[Vec<String>], content: &str) -> Event {
        let serialized = serde_json::to_string(&serde_json::json!([
            0,
            self.pubkey_hex,
            created_at,
            kind,
            tags,
            content,
        ]))
        .unwrap();

        let mut hasher = Sha256::new();
        hasher.update(serialized.as_bytes());
        let hash: [u8; 32] = hasher.finalize().into();
        let id = hex::encode(hash);

        let secp = Secp256k1::new();
        let msg = Message::from_digest(hash);
        let sig = secp.sign_schnorr(&msg, &self.keypair);
        let sig_hex = hex::encode(sig.serialize());

        Event {
            id,
            pubkey: self.pubkey_hex.clone(),
            created_at,
            kind,
            tags: tags.to_vec(),
            content: content.to_string(),
            sig: sig_hex,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct Event {
    id: String,
    pubkey: String,
    created_at: u64,
    kind: u64,
    tags: Vec<Vec<String>>,
    content: String,
    sig: String,
}

fn start_relay(port: u16) {
    let dir = format!("/tmp/nostrd-itest-{}", port);
    let _ = std::fs::remove_dir_all(&dir);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = nostrd::config::Config {
                listen_addr: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                nip42_enabled: false,
                ..Default::default()
            };
            let store = nostrd::db::LmdbStore::open(&std::path::PathBuf::from(&dir)).unwrap();
            nostrd::server::run(config, store).await.unwrap();
        });
    });
    std::thread::sleep(Duration::from_secs(1));
}

type WsConn = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

fn connect_relay(port: u16) -> WsConn {
    let (ws, _) =
        tungstenite::connect(format!("ws://127.0.0.1:{}", port)).expect("Failed to connect");
    ws
}

fn json_msg(data: &serde_json::Value) -> WsMsg {
    WsMsg::Text(serde_json::to_string(data).unwrap())
}

fn recv_json(ws: &mut WsConn) -> serde_json::Value {
    loop {
        match ws.read().unwrap() {
            WsMsg::Text(t) => return serde_json::from_str(&t).unwrap(),
            WsMsg::Ping(_) | WsMsg::Pong(_) | WsMsg::Close(_) => {}
            WsMsg::Binary(_) => {}
            WsMsg::Frame(_) => {}
        }
    }
}

#[test]
fn test_event_submit_and_query() {
    let port = 19876;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let event = key.sign_event(now, 1, &[], "hello world");
    let event_value = serde_json::to_value(&event).unwrap();
    ws.send(json_msg(&serde_json::json!(["EVENT", event_value])))
        .unwrap();

    let resp = recv_json(&mut ws);
    assert_eq!(resp[0], "OK");
    assert_eq!(resp[1], event.id);
    assert!(resp[2].as_bool().unwrap_or(false));

    let req = serde_json::json!(["REQ", "sub1", {"kinds": [1], "limit": 10}]);
    ws.send(json_msg(&req)).unwrap();

    let resp = recv_json(&mut ws);
    assert_eq!(resp[0], "EVENT");
    assert_eq!(resp[1], "sub1");
    let returned: Event = serde_json::from_value(resp[2].clone()).unwrap();
    assert_eq!(returned.id, event.id);

    let resp = recv_json(&mut ws);
    assert_eq!(resp[0], "EOSE");
    assert_eq!(resp[1], "sub1");
}

#[test]
fn test_relay_info_http() {
    let port = 19877;
    start_relay(port);

    let resp = ureq::get(&format!("http://127.0.0.1:{}", port))
        .set("Accept", "application/nostr+json")
        .call()
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.header("Content-Type").unwrap(),
        "application/nostr+json"
    );

    let body: serde_json::Value = resp.into_json().unwrap();
    assert_eq!(body["name"], "nostrd");
    assert!(body["supported_nips"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!(1)));
}

#[test]
fn test_broadcast_to_subscribers() {
    let port = 19878;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut sub_ws = connect_relay(port);
    let req = serde_json::json!(["REQ", "sub1", {"kinds": [1], "limit": 10}]);
    sub_ws.send(json_msg(&req)).unwrap();
    let eose = recv_json(&mut sub_ws);
    assert_eq!(eose[0], "EOSE");

    let mut pub_ws = connect_relay(port);
    let event = key.sign_event(now + 1, 1, &[], "broadcast test");
    pub_ws
        .send(json_msg(&serde_json::json!([
            "EVENT",
            serde_json::to_value(&event).unwrap()
        ])))
        .unwrap();

    let ok = recv_json(&mut pub_ws);
    assert!(ok[2].as_bool().unwrap_or(false));

    let evt = recv_json(&mut sub_ws);
    assert_eq!(evt[0], "EVENT");
    assert_eq!(evt[1], "sub1");
    let returned: Event = serde_json::from_value(evt[2].clone()).unwrap();
    assert_eq!(returned.id, event.id);
}

#[test]
fn test_count_message() {
    let port = 19879;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    for i in 0..3 {
        let event = key.sign_event(now + i, 1, &[], &format!("msg {}", i));
        ws.send(json_msg(&serde_json::json!([
            "EVENT",
            serde_json::to_value(&event).unwrap()
        ])))
        .unwrap();
        let ok = recv_json(&mut ws);
        assert!(ok[2].as_bool().unwrap_or(false));
    }

    ws.send(json_msg(
        &serde_json::json!(["COUNT", "c1", {"kinds": [1], "authors": [key.pubkey_hex]}]),
    ))
    .unwrap();
    let count_resp = recv_json(&mut ws);
    assert_eq!(count_resp[0], "COUNT");
    assert_eq!(count_resp[1], "c1");
    assert_eq!(count_resp[2]["count"], 3);
}

#[test]
fn test_replaceable_events() {
    let port = 19880;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let event1 = key.sign_event(now, 0, &[], "name: Alice");
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&event1).unwrap()
    ])))
    .unwrap();
    let ok1 = recv_json(&mut ws);
    assert!(ok1[2].as_bool().unwrap_or(false));

    let event2 = key.sign_event(now + 1, 0, &[], "name: Bob");
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&event2).unwrap()
    ])))
    .unwrap();
    let ok2 = recv_json(&mut ws);
    assert!(ok2[2].as_bool().unwrap_or(false));
    assert!(ok2[3].as_str().unwrap().starts_with("replaced:"));

    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub1",
        {"kinds": [0], "authors": [key.pubkey_hex]}
    ])))
    .unwrap();
    let evt = recv_json(&mut ws);
    assert_eq!(evt[0], "EVENT");
    let returned: Event = serde_json::from_value(evt[2].clone()).unwrap();
    assert_eq!(returned.id, event2.id);
    let eose = recv_json(&mut ws);
    assert_eq!(eose[0], "EOSE");
}

#[test]
fn test_duplicate_rejection() {
    let port = 19881;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let event = key.sign_event(now, 1, &[], "test duplicate");
    let event_val = serde_json::to_value(&event).unwrap();

    ws.send(json_msg(&serde_json::json!(["EVENT", event_val.clone()])))
        .unwrap();
    let ok1 = recv_json(&mut ws);
    assert!(ok1[2].as_bool().unwrap_or(false));

    ws.send(json_msg(&serde_json::json!(["EVENT", event_val])))
        .unwrap();
    let ok2 = recv_json(&mut ws);
    assert!(ok2[2].as_bool().unwrap_or(false));
    assert!(ok2[3].as_str().unwrap().starts_with("duplicate:"));
}

#[test]
fn test_ephemeral_event() {
    let port = 19882;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut sub_ws = connect_relay(port);
    sub_ws
        .send(json_msg(
            &serde_json::json!(["REQ", "sub1", {"kinds": [22222]}]),
        ))
        .unwrap();
    recv_json(&mut sub_ws); // EOSE

    let mut pub_ws = connect_relay(port);
    let event = key.sign_event(now, 22222, &[], "ephemeral test");
    pub_ws
        .send(json_msg(&serde_json::json!([
            "EVENT",
            serde_json::to_value(&event).unwrap()
        ])))
        .unwrap();
    let ok = recv_json(&mut pub_ws);
    assert!(ok[2].as_bool().unwrap_or(false));
    assert!(ok[3].as_str().unwrap().contains("ephemeral"));

    let evt = recv_json(&mut sub_ws);
    assert_eq!(evt[0], "EVENT");
    let returned: Event = serde_json::from_value(evt[2].clone()).unwrap();
    assert_eq!(returned.id, event.id);
}

#[test]
fn test_auth_flow() {
    let port = 19883;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let auth_event = key.sign_event(
        now,
        22242,
        &[
            vec!["challenge".to_string(), "none".to_string()],
            vec!["relay".to_string(), "ws://127.0.0.1:19883".to_string()],
        ],
        "",
    );
    ws.send(json_msg(&serde_json::json!([
        "AUTH",
        serde_json::to_value(&auth_event).unwrap()
    ])))
    .unwrap();
    let resp = recv_json(&mut ws);
    assert_eq!(resp[0], "NOTICE");
    assert!(resp[1].as_str().unwrap().contains("not supported"));
}

#[test]
fn test_protected_event_filtering() {
    let port = 19884;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut pub_ws = connect_relay(port);

    let event = key.sign_event(now, 1, &[vec!["-".to_string()]], "protected");
    pub_ws
        .send(json_msg(&serde_json::json!([
            "EVENT",
            serde_json::to_value(&event).unwrap()
        ])))
        .unwrap();
    let ok = recv_json(&mut pub_ws);
    assert!(ok[2].as_bool().unwrap_or(false));

    let mut sub_ws = connect_relay(port);
    sub_ws
        .send(json_msg(
            &serde_json::json!(["REQ", "sub1", {"kinds": [1]}]),
        ))
        .unwrap();
    let resp = recv_json(&mut sub_ws);
    assert_eq!(resp[0], "EOSE");
    assert_eq!(resp[1], "sub1");
}

#[test]
fn test_req_no_filters_returns_closed() {
    let port = 19885;
    start_relay(port);

    let mut ws = connect_relay(port);

    ws.send(json_msg(&serde_json::json!(["REQ", "sub1"])))
        .unwrap();
    let resp = recv_json(&mut ws);
    // Should receive EOSE or CLOSED for empty filter REQ
    assert!(
        resp[0] == "EOSE" || resp[0] == "CLOSED" || resp[0] == "NOTICE",
        "Expected EOSE/CLOSED/NOTICE, got {}",
        resp[0]
    );
    assert_eq!(resp[1], "sub1");
}

#[test]
fn test_invalid_event_signature_rejected() {
    let port = 19887;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let mut event = key.sign_event(now, 1, &[], "test");
    event.sig = "00".repeat(64);

    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&event).unwrap()
    ])))
    .unwrap();
    let ok = recv_json(&mut ws);
    assert_eq!(ok[0], "OK");
    assert!(!ok[2].as_bool().unwrap_or(true));
}

#[test]
fn test_too_many_filters_returns_closed() {
    let port = 19888;
    start_relay(port);

    let mut ws = connect_relay(port);

    let mut filters = Vec::new();
    for _ in 0..11 {
        filters.push(serde_json::json!({"kinds": [1]}));
    }
    let mut req = vec![serde_json::json!("REQ"), serde_json::json!("sub1")];
    req.extend(filters);

    ws.send(json_msg(&serde_json::json!(req))).unwrap();
    let resp = recv_json(&mut ws);
    assert_eq!(resp[0], "CLOSED");
    assert!(resp[2].as_str().unwrap().contains("too many filters"));
}

#[test]
fn test_deletion_event_lifecycle() {
    let port = 19889;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    // Publish a regular event
    let event1 = key.sign_event(now, 1, &[], "to be deleted");
    let evt1_val = serde_json::to_value(&event1).unwrap();
    ws.send(json_msg(&serde_json::json!(["EVENT", evt1_val])))
        .unwrap();
    let ok = recv_json(&mut ws);
    assert!(ok[2].as_bool().unwrap_or(false));

    // Verify it exists
    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub1",
        {"ids": [event1.id.clone()]}
    ])))
    .unwrap();
    let evt = recv_json(&mut ws);
    assert_eq!(evt[0], "EVENT");
    let _eose = recv_json(&mut ws);
    assert_eq!(_eose[0], "EOSE");

    // Publish deletion event (kind 5)
    let del_event = key.sign_event(
        now + 1,
        5,
        &[vec!["e".to_string(), event1.id.clone()]],
        "deleting",
    );
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&del_event).unwrap()
    ])))
    .unwrap();
    let ok = recv_json(&mut ws);
    assert!(ok[2].as_bool().unwrap_or(false));

    // Verify original event is gone
    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub2",
        {"ids": [event1.id]}
    ])))
    .unwrap();
    let resp = recv_json(&mut ws);
    assert_eq!(resp[0], "EOSE");
}

#[test]
fn test_multi_filter_dedup() {
    let port = 19890;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    // Publish one event
    let event = key.sign_event(now, 1, &[], "dedup test");
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&event).unwrap()
    ])))
    .unwrap();
    recv_json(&mut ws); // OK

    // Subscribe with two filters that both match the event
    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub1",
        {"kinds": [1]},
        {"authors": [key.pubkey_hex]}
    ])))
    .unwrap();

    // Should receive event only ONCE (dedup)
    let evt = recv_json(&mut ws);
    assert_eq!(evt[0], "EVENT");
    let eose = recv_json(&mut ws);
    // If only EOSE arrives (no second EVENT), dedup is working
    assert_eq!(eose[0], "EOSE");
}

#[test]
fn test_reaction_requires_e_tag() {
    let port = 19781;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let event = key.sign_event(now, 7, &[], "+");
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&event).unwrap()
    ])))
    .unwrap();

    let ok = recv_json(&mut ws);
    assert_eq!(ok[0], "OK");
    assert!(!ok[2].as_bool().unwrap_or(true));
    assert!(ok[3].as_str().unwrap().contains("e"));
}

#[test]
fn test_reaction_with_e_tag_accepted() {
    let port = 19782;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let target_id = "a".repeat(64);
    let reaction = key.sign_event(
        now,
        7,
        &[
            vec!["e".to_string(), target_id.clone()],
            vec!["p".to_string(), key.pubkey_hex.clone()],
            vec!["k".to_string(), "1".to_string()],
        ],
        "+",
    );
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&reaction).unwrap()
    ])))
    .unwrap();

    let ok = recv_json(&mut ws);
    assert!(ok[2].as_bool().unwrap_or(false));

    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub1",
        {"#e": [target_id], "kinds": [7]}
    ])))
    .unwrap();
    let evt = recv_json(&mut ws);
    assert_eq!(evt[0], "EVENT");
    let returned: Event = serde_json::from_value(evt[2].clone()).unwrap();
    assert_eq!(returned.id, reaction.id);
    let eose = recv_json(&mut ws);
    assert_eq!(eose[0], "EOSE");
}

#[test]
fn test_reaction_content_variants() {
    let port = 19783;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let target_id = "b".repeat(64);
    let contents = ["+", "-", "👍", "👎"];

    let mut ws = connect_relay(port);

    for (i, content) in contents.iter().enumerate() {
        let reaction = key.sign_event(
            now + i as u64,
            7,
            &[
                vec!["e".to_string(), target_id.clone()],
                vec!["p".to_string(), key.pubkey_hex.clone()],
            ],
            content,
        );
        ws.send(json_msg(&serde_json::json!([
            "EVENT",
            serde_json::to_value(&reaction).unwrap()
        ])))
        .unwrap();
        let ok = recv_json(&mut ws);
        assert!(ok[2].as_bool().unwrap_or(false), "reaction {} failed", content);
    }

    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub1",
        {"#e": [target_id], "kinds": [7]}
    ])))
    .unwrap();

    let mut count = 0;
    loop {
        let resp = recv_json(&mut ws);
        if resp[0] == "EOSE" {
            break;
        }
        assert_eq!(resp[0], "EVENT");
        count += 1;
    }
    assert_eq!(count, 4);
}

#[test]
fn test_external_reaction_requires_k_and_i_tags() {
    let port = 19784;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let event = key.sign_event(now, 17, &[], "⭐");
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&event).unwrap()
    ])))
    .unwrap();

    let ok = recv_json(&mut ws);
    assert_eq!(ok[0], "OK");
    assert!(!ok[2].as_bool().unwrap_or(true));
    assert!(ok[3].as_str().unwrap().contains("k") || ok[3].as_str().unwrap().contains("i"));
}

#[test]
fn test_external_reaction_with_k_and_i_accepted() {
    let port = 19785;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let reaction = key.sign_event(
        now,
        17,
        &[
            vec!["k".to_string(), "web".to_string()],
            vec!["i".to_string(), "https://example.com".to_string()],
        ],
        "⭐",
    );
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&reaction).unwrap()
    ])))
    .unwrap();

    let ok = recv_json(&mut ws);
    assert!(ok[2].as_bool().unwrap_or(false));

    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub1",
        {"kinds": [17]}
    ])))
    .unwrap();
    let evt = recv_json(&mut ws);
    assert_eq!(evt[0], "EVENT");
    let returned: Event = serde_json::from_value(evt[2].clone()).unwrap();
    assert_eq!(returned.id, reaction.id);
    let eose = recv_json(&mut ws);
    assert_eq!(eose[0], "EOSE");
}

#[test]
fn test_generic_tag_filter() {
    let port = 19891;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    // Publish event with hashtag
    let event = key.sign_event(
        now,
        1,
        &[vec!["t".to_string(), "nostr".to_string()]],
        "test hashtag",
    );
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&event).unwrap()
    ])))
    .unwrap();
    recv_json(&mut ws); // OK

    // Query with generic #t filter
    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub1",
        {"#t": ["nostr"]}
    ])))
    .unwrap();
    let evt = recv_json(&mut ws);
    assert_eq!(evt[0], "EVENT");
    let returned: Event = serde_json::from_value(evt[2].clone()).unwrap();
    assert_eq!(returned.id, event.id);
    let eose = recv_json(&mut ws);
    assert_eq!(eose[0], "EOSE");
}

#[test]
fn test_repost_requires_e_tag() {
    let port = 19786;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let event = key.sign_event(now, 6, &[], "");
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&event).unwrap()
    ])))
    .unwrap();

    let ok = recv_json(&mut ws);
    assert_eq!(ok[0], "OK");
    assert!(!ok[2].as_bool().unwrap_or(true));
    assert!(ok[3].as_str().unwrap().contains("e"));
}

#[test]
fn test_repost_with_e_tag_accepted() {
    let port = 19787;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let target_id = "c".repeat(64);
    let repost = key.sign_event(
        now,
        6,
        &[
            vec!["e".to_string(), target_id.clone(), "wss://relay.example.com".to_string()],
            vec!["p".to_string(), key.pubkey_hex.clone()],
        ],
        r#"{"id":"...","pubkey":"...","content":"hello"}"#,
    );
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&repost).unwrap()
    ])))
    .unwrap();

    let ok = recv_json(&mut ws);
    assert!(ok[2].as_bool().unwrap_or(false));

    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub1",
        {"#e": [target_id], "kinds": [6]}
    ])))
    .unwrap();
    let evt = recv_json(&mut ws);
    assert_eq!(evt[0], "EVENT");
    let returned: Event = serde_json::from_value(evt[2].clone()).unwrap();
    assert_eq!(returned.id, repost.id);
    let eose = recv_json(&mut ws);
    assert_eq!(eose[0], "EOSE");
}

#[test]
fn test_generic_repost_kind16_accepted() {
    let port = 19788;
    start_relay(port);

    let key = Signer::generate();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut ws = connect_relay(port);

    let target_id = "d".repeat(64);
    let repost = key.sign_event(
        now,
        16,
        &[
            vec!["e".to_string(), target_id.clone()],
            vec!["k".to_string(), "30023".to_string()],
        ],
        "generic repost",
    );
    ws.send(json_msg(&serde_json::json!([
        "EVENT",
        serde_json::to_value(&repost).unwrap()
    ])))
    .unwrap();

    let ok = recv_json(&mut ws);
    assert!(ok[2].as_bool().unwrap_or(false));

    ws.send(json_msg(&serde_json::json!([
        "REQ",
        "sub1",
        {"kinds": [16]}
    ])))
    .unwrap();
    let evt = recv_json(&mut ws);
    assert_eq!(evt[0], "EVENT");
    let returned: Event = serde_json::from_value(evt[2].clone()).unwrap();
    assert_eq!(returned.id, repost.id);
    let eose = recv_json(&mut ws);
    assert_eq!(eose[0], "EOSE");
}
