use nostr_sdk::{Timestamp, Client, Options, Filter, PublicKey, Kind, SubscriptionId, RelayPoolNotification};
use std::error::Error;
use crate::boosts::Boostagram;
use nostr_sdk::prelude::Output;
use serde::{Serialize, Deserialize};
use serde_json;

use std::future::Future;

#[derive(Serialize, Deserialize, Debug)]
struct BoostBoardEvent {
    boostagram: Option<Boostagram>,
}

pub struct BoostBoard {
    client: Client,
    pubkey: PublicKey,
}

impl BoostBoard {
    pub async fn new(relay_addr: &str, pubkey: &str) -> Result<Self, Box<dyn Error>> {
        let opts = Options::new().wait_for_send(false);
        let client = Client::builder().opts(opts).build();

        client.add_relay(relay_addr).await?;
        client.connect().await;

        let pubkey = PublicKey::from_hex(&pubkey)?;

        Ok(Self {
            client,
            pubkey,
        })
    }

    pub async fn subscribe(&self, since: Option<Timestamp>) -> Result<SubscriptionId, Box<dyn Error>> {
        let ts = match since {
            Some(ts) => ts,
            None => Timestamp::from_secs(0),
        };

        let subscription = Filter::new()
            .author(self.pubkey)
            .kind(Kind::ApplicationSpecificData)
            .since(ts);

        // Subscribe (auto generate subscription ID)
        let Output { val: sub_id_1, .. } = self.client.subscribe(vec![subscription], None).await?;

        Ok(sub_id_1)
    }

    pub async fn handle_boosts<F, Fut>(&self, sub_id: SubscriptionId, func: F) -> Result<(), Box<dyn Error>>
    where
        F: Fn(Boostagram) -> Fut,
        Fut: Future<Output = ()>,
    {

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

                if let Ok(BoostBoardEvent { boostagram: Some(boost), .. }) = serde_json::from_str(&event.content) {
                    func(boost).await;
                }

            }
            Ok(false) // Set to true to exit from the loop
        })
        .await?;

        Ok(())
    }
}