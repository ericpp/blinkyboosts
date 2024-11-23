use lightning_invoice::Bolt11Invoice;

use nostr_sdk::nips::nip01::Coordinate;
use nostr_sdk::prelude::Output;
use nostr_sdk::{Timestamp, Client, Options, Filter, Kind, SubscriptionId, RelayPoolNotification, JsonUtil, TagKind};

use serde::{Serialize, Deserialize};
use serde_json::Value;

use std::error::Error;
use std::future::Future;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Zap {
    pub sender_name:      Option<String>,
    pub message:          Option<String>,
    pub value_msat_total: i64,
}

#[derive(Debug)]
pub struct Zaps {
    client: Client,
    naddr: Coordinate,
}

impl Zaps {
    pub async fn new(relay_addrs: &Vec<String>, naddr: &str) -> Result<Self, Box<dyn Error>> {
        let opts = Options::new().wait_for_send(false);
        let client = Client::builder().opts(opts).build();

        for relay_addr in relay_addrs {
            client.add_relay(relay_addr).await?;
        }

        client.connect().await;

        let naddr: Coordinate = Coordinate::parse(naddr).unwrap();

        Ok(Self {
            client,
            naddr,
        })
    }

    pub async fn subscribe(&self, since: Option<Timestamp>) -> Result<SubscriptionId, Box<dyn Error>> {
        let ts = match since {
            Some(ts) => ts,
            None => Timestamp::from_secs(0),
        };

        let subscription = Filter::new()
            .coordinate(&self.naddr)
            .kind(Kind::ZapReceipt)
            .since(ts);

        // Subscribe (auto generate subscription ID)
        let Output { val: sub_id_1, .. } = self.client.subscribe(vec![subscription], None).await?;

        Ok(sub_id_1)
    }

    pub async fn subscribe_zaps<F, Fut>(&self, since: Option<Timestamp>, func: F) -> Result<(), Box<dyn Error>>
    where
     F: Fn(Zap) -> Fut,
     Fut: Future<Output = ()>,
    {
        let sub_id = self.subscribe(since).await?;

        // Handle subscription notifications with `handle_notifications` method
        self.client.handle_notifications(|notification| async {
            if let RelayPoolNotification::Event {
                subscription_id,
                event,
                ..
            } = notification
            {
                // Check subscription ID
                if subscription_id != sub_id {
                    return Ok(false);
                }

                let mut description = String::new();
                let mut bolt11 = String::new();

                for tag in &event.tags {
                    let content = tag.content().unwrap_or_default().to_string();

                    if tag.kind() == TagKind::Description {
                        description = content;
                    }
                    else if tag.kind() == TagKind::Bolt11 {
                        bolt11 = content;
                    }
                }

                let value_msat_total = if !bolt11.is_empty() {
                    match bolt11.parse::<Bolt11Invoice>() {
                        Ok(invoice) => invoice.amount_milli_satoshis().unwrap_or_default() as i64,
                        Err(_) => 0
                    }
                }
                else {
                    0
                };

                let mut pubkey = String::new();

                if let Ok(Value::Object(req)) = serde_json::from_str(&description) {
                    if let Value::String(pk) = &req["pubkey"] {
                        pubkey = pk.clone();
                    }
                }

                let result =  Zap {
                    sender_name: Some(pubkey),
                    message: Some(event.content),
                    value_msat_total,
                };

                func(result).await;
            }
            Ok(false) // Set to true to exit from the loop
        })
        .await?;

        Ok(())
    }
}