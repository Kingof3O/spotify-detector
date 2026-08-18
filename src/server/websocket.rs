use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures_util::StreamExt;

use crate::media::MediaState;

use super::state::AppState;

pub async fn upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| client(socket, state))
}

async fn client(mut socket: WebSocket, state: AppState) {
    tracing::debug!("OBS WebSocket client connected");
    let mut media = state.media.clone();

    let initial = media.borrow().clone();
    if !send_state(&mut socket, &initial).await {
        tracing::debug!("OBS WebSocket client disconnected before initial state");
        return;
    }

    loop {
        tokio::select! {
            changed = media.changed() => {
                if changed.is_err() {
                    break;
                }
                let next = media.borrow_and_update().clone();
                if !send_state(&mut socket, &next).await {
                    break;
                }
            }
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) | Some(Ok(Message::Pong(_))) => {
                        // V1 is intentionally read-only. Ignore browser messages rather than
                        // exposing a command surface to the local page.
                    }
                    Some(Err(error)) => {
                        tracing::debug!(?error, "OBS WebSocket read failed");
                        break;
                    }
                }
            }
        }
    }

    tracing::debug!("OBS WebSocket client disconnected");
}

async fn send_state(socket: &mut WebSocket, state: &MediaState) -> bool {
    let payload = match serde_json::to_string(state) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(?error, "could not serialize media state");
            return false;
        }
    };

    socket.send(Message::Text(payload.into())).await.is_ok()
}
