use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde_derive::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// One effect slot from a `hello` or `effects` snapshot.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffectSlotState {
    pub id: String,
    pub unlocked: bool,
    pub remaining_ms: u64,
}

/// A fired effect broadcast to every connected client.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FiredEffect {
    pub id: String,
    pub label: String,
    pub sender_name: String,
    pub message: String,
}

/// A message from the SplitKit live socket. Unknown types are ignored.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMessage {
    Hello { effects: Vec<EffectSlotState> },
    Effects { effects: Vec<EffectSlotState> },
    Effect { effect: FiredEffect },
    #[serde(other)]
    Unknown,
}

/// The server closed the socket because live mode is off or the database is
/// not configured. Retrying immediately will not help.
#[derive(Debug)]
pub struct LiveDisabled;

impl std::fmt::Display for LiveDisabled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "live mode disabled")
    }
}

impl std::error::Error for LiveDisabled {}

/// Connect to the live socket and call `on_effect` for every fired effect.
///
/// Returns when the socket closes. A close with code 1008 and reason
/// `disabled` returns a [`LiveDisabled`] error.
pub async fn subscribe<F, Fut>(url: &str, mut on_effect: F) -> Result<()>
where
    F: FnMut(FiredEffect) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let (socket, _) = connect_async(url)
        .await
        .with_context(|| format!("Failed to connect to SplitKit at {}", url))?;
    let (_, mut reader) = socket.split();

    while let Some(message) = reader.next().await {
        let message = message.context("SplitKit connection lost")?;
        match message {
            Message::Text(text) => match serde_json::from_str::<ServerMessage>(&text) {
                Ok(ServerMessage::Effect { effect }) => on_effect(effect).await,
                Ok(ServerMessage::Hello { effects }) => {
                    println!("SplitKit hello: {}", slot_summary(&effects));
                }
                Ok(ServerMessage::Effects { effects }) => {
                    println!("SplitKit effects: {}", slot_summary(&effects));
                }
                Ok(ServerMessage::Unknown) => {}
                Err(e) => eprintln!("Ignoring malformed SplitKit message: {:#}", e),
            },
            Message::Close(frame) => {
                if let Some(frame) = frame {
                    if frame.code == 1008.into() && frame.reason == "disabled" {
                        bail!(LiveDisabled);
                    }
                    bail!("SplitKit closed the socket: {} {}", frame.code, frame.reason);
                }
                bail!("SplitKit closed the socket");
            }
            _ => {}
        }
    }

    bail!("SplitKit connection ended")
}

fn slot_summary(effects: &[EffectSlotState]) -> String {
    if effects.is_empty() {
        return "no effects".to_string();
    }
    effects
        .iter()
        .map(|slot| {
            if slot.unlocked {
                format!("{} unlocked", slot.id)
            } else {
                format!("{} locked ({}ms)", slot.id, slot.remaining_ms)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hello() {
        let message: ServerMessage = serde_json::from_str(
            r#"{
                "type": "hello",
                "effects": [
                    { "id": "wide", "unlocked": false, "remainingMs": 60000 },
                    { "id": "dj", "unlocked": true, "remainingMs": 0 }
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(
            message,
            ServerMessage::Hello {
                effects: vec![
                    EffectSlotState { id: "wide".into(), unlocked: false, remaining_ms: 60000 },
                    EffectSlotState { id: "dj".into(), unlocked: true, remaining_ms: 0 },
                ]
            }
        );
    }

    #[test]
    fn parses_effects() {
        let message: ServerMessage = serde_json::from_str(
            r#"{
                "type": "effects",
                "effects": [
                    { "id": "bubbles", "unlocked": false, "remainingMs": 12000 }
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(
            message,
            ServerMessage::Effects {
                effects: vec![EffectSlotState {
                    id: "bubbles".into(),
                    unlocked: false,
                    remaining_ms: 12000
                }]
            }
        );
    }

    #[test]
    fn parses_fired_effect() {
        let message: ServerMessage = serde_json::from_str(
            r#"{
                "type": "effect",
                "effect": {
                    "id": "dj",
                    "label": "DJ",
                    "senderName": "anonymous",
                    "message": "drop the beat"
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            message,
            ServerMessage::Effect {
                effect: FiredEffect {
                    id: "dj".into(),
                    label: "DJ".into(),
                    sender_name: "anonymous".into(),
                    message: "drop the beat".into(),
                }
            }
        );
    }

    #[test]
    fn unknown_message_type_is_ignored() {
        let message: ServerMessage = serde_json::from_str(r#"{ "type": "surprise" }"#).unwrap();
        assert_eq!(message, ServerMessage::Unknown);
    }
}
