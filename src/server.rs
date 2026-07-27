use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, Semaphore};
use tokio::time::Duration;

use crate::config::Config;
use crate::db::LmdbStore;
use crate::nips::{self, process_message, Action, NipContext};
use crate::types::{parse_client_message, ClientMessage, Event, RelayMessage};
use crate::utils::crypto;
use crate::ws::session::SessionManager;

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    store: Arc<LmdbStore>,
    sessions: Arc<SessionManager>,
    relay_info: Arc<crate::nips::nip11::RelayInfo>,
    event_broadcast: broadcast::Sender<Arc<Event>>,
    conn_semaphore: Arc<Semaphore>,
}

async fn root_handler(
    ws: Option<WebSocketUpgrade>,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    if let Some(ws) = ws {
        let max_size = state.config.max_ws_message_size;
        ws.max_message_size(max_size)
            .on_upgrade(move |socket| async move {
                let permit = match state.conn_semaphore.try_acquire() {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::warn!("Connection limit reached, rejecting new connection");
                        let _ = socket.close().await;
                        return;
                    }
                };
                handle_socket(socket, state.clone()).await;
                drop(permit);
            })
            .into_response()
    } else {
        handle_http(headers, &state.relay_info).await
    }
}

async fn handle_http(
    headers: axum::http::HeaderMap,
    info: &Arc<crate::nips::nip11::RelayInfo>,
) -> axum::response::Response {
    let accept = headers
        .get("Accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !accept.is_empty() && !accept.contains("application/nostr+json") && !accept.contains("*/*") {
        return axum::http::StatusCode::NOT_ACCEPTABLE.into_response();
    }

    let body = serde_json::to_string(&**info).unwrap_or_default();
    match axum::response::Response::builder()
        .header("Content-Type", "application/nostr+json")
        .header("Access-Control-Allow-Origin", "*")
        .body(axum::body::Body::from(body))
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Failed to build HTTP response: {}", e);
            axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

struct ConnectionCtx {
    config: Arc<Config>,
    store: Arc<LmdbStore>,
    sessions: Arc<SessionManager>,
    event_broadcast: broadcast::Sender<Arc<Event>>,
    session_id: String,
    neg_sessions: crate::nips::nip77::NegentropySessions,
}

impl ConnectionCtx {
    async fn auth_pubkey(&self) -> Option<[u8; 32]> {
        self.sessions
            .get(&self.session_id)
            .await
            .and_then(|s| s.auth_pubkey)
            .and_then(|pk| hex::decode(pk).ok())
            .and_then(|bytes| bytes.try_into().ok())
    }

    async fn set_auth_pubkey(&self, pk: String) {
        self.sessions
            .update(&self.session_id, |s| {
                s.auth_pubkey = Some(pk);
                s.auth_challenge = None;
            })
            .await;
    }

    async fn subscription_count(&self) -> usize {
        self.sessions
            .get(&self.session_id)
            .await
            .map(|s| s.subscriptions.len())
            .unwrap_or(0)
    }

    async fn add_subscription(&self, sub_id: String, filters: Vec<crate::types::Filter>) {
        self.sessions
            .update(&self.session_id, |s| {
                s.add_subscription(sub_id, filters);
            })
            .await;
    }

    async fn remove_subscription(&self, sub_id: &str) {
        self.sessions
            .update(&self.session_id, |s| {
                s.remove_subscription(sub_id);
            })
            .await;
    }
}

async fn handle_socket(ws: WebSocket, state: AppState) {
    let session_id = uuid::Uuid::new_v4().to_string();
    if state.sessions.try_create(&session_id, state.config.max_sessions).await.is_none() {
        tracing::warn!("Session limit reached ({})", state.config.max_sessions);
        let (mut ws_sender, _) = ws.split();
        let notice = RelayMessage::notice("error: server at capacity, try again later");
        let _ = ws_sender.send(Message::Text(notice.to_json())).await;
        let _ = ws_sender.close().await;
        return;
    }

    let (mut ws_sender, mut ws_receiver) = ws.split();
    let mut broadcast_rx = state.event_broadcast.subscribe();

    let ctx = ConnectionCtx {
        config: state.config.clone(),
        store: state.store.clone(),
        sessions: state.sessions.clone(),
        event_broadcast: state.event_broadcast.clone(),
        session_id: session_id.clone(),
        neg_sessions: crate::nips::nip77::NegentropySessions::new(),
    };

    if state.config.nip42_enabled {
        let challenge = nips::nip42::generate_challenge();
        state
            .sessions
            .update(&session_id, |s| {
                s.auth_challenge = Some(challenge.clone());
            })
            .await;
        let msg = RelayMessage::auth(&challenge).to_json();
        let _ = ws_sender.send(Message::Text(msg)).await;
    }

    let nip_ctx = NipContext {
        config: ctx.config.clone(),
        store: ctx.store.clone(),
    };

    let timeout_dur = Duration::from_secs(state.config.connection_timeout_secs);
    let mut idle_timer = Box::pin(tokio::time::sleep(timeout_dur));

    loop {
        let mut msg_fut = ws_receiver.next();
        let mut broadcast_fut = Box::pin(broadcast_rx.recv());

        tokio::select! {
            msg = &mut msg_fut => {
                idle_timer.as_mut().reset(tokio::time::Instant::now() + timeout_dur);
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let text_str = text.to_string();
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            parse_client_message(&text_str)
                        }));
                        match result {
                            Ok(Some(client_msg)) => {
                                if !process_client_msg(
                                    &nip_ctx,
                                    &ctx,
                                    &client_msg,
                                    &mut ws_sender,
                                ).await {
                                    break;
                                }
                            }
                            Ok(None) => {
                                let notice = RelayMessage::notice("error: invalid message format");
                                let _ = ws_sender.send(Message::Text(notice.to_json())).await;
                            }
                            Err(_) => {
                                tracing::error!("Panic during message parsing for connection {}", ctx.session_id);
                                let notice = RelayMessage::notice("error: internal server error");
                                let _ = ws_sender.send(Message::Text(notice.to_json())).await;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = ws_sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    _ => {}
                }
            }
            event = &mut broadcast_fut => {
                idle_timer.as_mut().reset(tokio::time::Instant::now() + timeout_dur);
                match event {
                    Ok(event) => {
                        handle_broadcast(&ctx, &event, &mut ws_sender).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Broadcast channel lagged by {} messages for {}", n, session_id);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = &mut idle_timer => {
                tracing::info!("Connection {} idle timeout after {}s", session_id, timeout_dur.as_secs());
                let notice = RelayMessage::notice("error: connection idle timeout");
                let _ = ws_sender.send(Message::Text(notice.to_json())).await;
                break;
            }
        }
    }

    state.sessions.remove(&session_id).await;
    tracing::info!("Connection {} closed", session_id);
}

async fn process_client_msg(
    nip_ctx: &NipContext,
    ctx: &ConnectionCtx,
    client_msg: &ClientMessage,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> bool {
    let auth_pubkey_bytes = ctx.auth_pubkey().await;

    if let ClientMessage::Auth { event } = client_msg {
        if !ctx.config.nip42_enabled {
            let notice = RelayMessage::notice("error: AUTH not supported by this relay");
            let _ = sender.send(Message::Text(notice.to_json())).await;
            return true;
        }
        let expected_challenge = ctx
            .sessions
            .get(&ctx.session_id)
            .await
            .and_then(|s| s.auth_challenge.clone());
        match expected_challenge {
            None => {
                let notice = RelayMessage::notice("error: AUTH without prior challenge");
                let _ = sender.send(Message::Text(notice.to_json())).await;
                return true;
            }
            Some(challenge) => {
                let relay_url = format!("ws://{}", ctx.config.listen_addr);
                match crypto::verify_auth_event(event, &challenge, &relay_url) {
                    Ok(false) | Err(_) => {
                        let notice = RelayMessage::notice("error: AUTH challenge verification failed");
                        let _ = sender.send(Message::Text(notice.to_json())).await;
                        return true;
                    }
                    Ok(true) => {}
                }
            }
        }
    }

    if ctx.config.auth_required
        && auth_pubkey_bytes.is_none()
        && !matches!(client_msg, ClientMessage::Auth { .. })
    {
        match client_msg {
            ClientMessage::Event { event } => {
                let msg = RelayMessage::ok(&event.id, false, "auth-required: this relay requires authentication");
                let _ = sender.send(Message::Text(msg.to_json())).await;
                if ctx.config.nip42_enabled {
                    let challenge = nips::nip42::generate_challenge();
                    ctx.sessions
                        .update(&ctx.session_id, |s| {
                            s.auth_challenge = Some(challenge.clone());
                        })
                        .await;
                    let auth_msg = RelayMessage::auth(&challenge).to_json();
                    let _ = sender.send(Message::Text(auth_msg)).await;
                }
                return true;
            }
            ClientMessage::Req { subscription_id, .. }
            | ClientMessage::Count { subscription_id, .. } => {
                let msg = RelayMessage::closed(subscription_id, "auth-required: this relay requires authentication");
                let _ = sender.send(Message::Text(msg.to_json())).await;
                if ctx.config.nip42_enabled {
                    let challenge = nips::nip42::generate_challenge();
                    ctx.sessions
                        .update(&ctx.session_id, |s| {
                            s.auth_challenge = Some(challenge.clone());
                        })
                        .await;
                    let auth_msg = RelayMessage::auth(&challenge).to_json();
                    let _ = sender.send(Message::Text(auth_msg)).await;
                }
                return true;
            }
            _ => {}
        }
    }

    if let ClientMessage::Req { .. } = client_msg {
        let current_count = ctx.subscription_count().await;
        let max_subs = ctx.config.max_subscriptions_per_client;
        if current_count >= max_subs {
            let notice = RelayMessage::notice(format!("error: too many subscriptions, max {}", max_subs));
            let _ = sender.send(Message::Text(notice.to_json())).await;
            return true;
        }
    }

    if let ClientMessage::Event { .. } = client_msg {
        let now = chrono::Utc::now().timestamp() as u64;
        let mut rate_limited = false;
        ctx.sessions
            .update(&ctx.session_id, |s| {
                rate_limited = !s.check_rate_limit(ctx.config.max_events_per_sec, now);
            })
            .await;
        if rate_limited {
            let notice = RelayMessage::notice("rate-limited: too many events, slow down");
            let _ = sender.send(Message::Text(notice.to_json())).await;
            return true;
        }
    }

    let had_parse_error = match client_msg {
        ClientMessage::Req { _parse_error, .. } | ClientMessage::Count { _parse_error, .. } => {
            *_parse_error
        }
        _ => false,
    };
    if had_parse_error {
        let notice = RelayMessage::notice("error: some filters were malformed and were ignored");
        let _ = sender.send(Message::Text(notice.to_json())).await;
    }

    match client_msg {
        ClientMessage::Req {
            subscription_id,
            filters,
            ..
        } => {
            ctx.add_subscription(subscription_id.clone(), filters.clone())
                .await;
        }
        ClientMessage::Close { subscription_id } => {
            ctx.remove_subscription(subscription_id).await;
        }
        _ => {}
    }

    let actions = process_message(
        nip_ctx,
        client_msg,
        auth_pubkey_bytes.as_ref(),
        &ctx.neg_sessions,
    );

    let mut set_auth_pk: Option<String> = None;
    for action in &actions {
        if let Action::SetAuth(pk) = action {
            set_auth_pk = Some(pk.clone());
        }
    }
    if let Some(pk) = set_auth_pk {
        ctx.set_auth_pubkey(pk).await;
    }

    for action in actions {
        match action {
            Action::Send(msg) => {
                if sender.send(Message::Text(msg.to_json())).await.is_err() {
                    return false;
                }
            }
            Action::CloseSubscription(sub_id, reason) => {
                let msg = RelayMessage::closed(&sub_id, &reason);
                let _ = sender.send(Message::Text(msg.to_json())).await;
                ctx.remove_subscription(&sub_id).await;
            }
            Action::SetAuth(_) => {}
            Action::BroadcastEvent(event) => {
                let _ = ctx.event_broadcast.send(event);
            }
        }
    }

    true
}

async fn handle_broadcast(
    ctx: &ConnectionCtx,
    event: &Arc<Event>,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) {
    let session = ctx.sessions.get(&ctx.session_id).await;
    let session = match session {
        Some(s) => s,
        None => return,
    };

    let auth_pubkey_bytes: Option<[u8; 32]> = session
        .auth_pubkey
        .as_ref()
        .and_then(|pk| hex::decode(pk).ok())
        .and_then(|bytes| bytes.try_into().ok());

    for (sub_id, filters) in &session.subscriptions {
        let mut matched = false;
        for filter in filters {
            if !matched && filter.matches_event(event) {
                matched = true;
                if event.is_protected() && auth_pubkey_bytes.is_none() {
                    break;
                }
                let msg = RelayMessage::event(sub_id, Arc::clone(event)).to_json();
                let _ = sender.send(Message::Text(msg)).await;
            }
        }
    }
}

pub async fn run(config: Config, store: Arc<LmdbStore>) -> Result<(), Box<dyn std::error::Error>> {
    let addr = config.listen_addr;
    let config = Arc::new(config);

    let relay_info = Arc::new(crate::nips::nip11::RelayInfo::from_config(&config));

    let (event_broadcast, _) = broadcast::channel(config.broadcast_channel_size);
    let conn_semaphore = Arc::new(Semaphore::new(config.max_connections));

    let sessions = SessionManager::new();
    let state = AppState {
        config: config.clone(),
        store: store.clone(),
        sessions: sessions.clone(),
        relay_info: relay_info.clone(),
        event_broadcast,
        conn_semaphore: conn_semaphore.clone(),
    };

    let app = Router::new()
        .route("/", get(root_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("nostrd relay listening on ws://{}", addr);
    tracing::info!(
        "NIP-11 relay info at http://{} (Accept: application/nostr+json)",
        addr
    );

    axum::serve(listener, app).await?;

    Ok(())
}
