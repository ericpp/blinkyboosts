use crate::boosts::Boostagram;

use hex;

use nostr_sdk::{Client, Filter, Keys, Kind};
use nostr_sdk::nips::nip04;
use nostr_sdk::nips::nip47;
use nostr_sdk::NWC as NostrWC;
use nostr_sdk::RelayPoolNotification;
use nostr_sdk::Timestamp;

use serde_json::Value;
use serde::Deserialize;

use std::future::Future;
use std::str::FromStr;
use anyhow::{Context, Result};

#[derive(Clone)]
pub struct NWC {
    client: Client,
    uri: nip47::NostrWalletConnectURI,
    // nwc: NostrWC,
    // uri_str: String,
}

#[derive(Deserialize, Debug)]
pub struct GetInfoResult {
    // pub alias: String,
    // pub color: String,
    // pub pubkey: String,
    // pub network: String,
    // pub block_height: u64,
    // pub block_hash: String,
    // pub methods: Vec<String>,
    pub notifications: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct PayNotification {
    // pub r#type: String,
    // pub invoice: String,
    // pub description: String,
    // pub description_hash: String,
    // pub preimage: String,
    // pub payment_hash: String,
    // pub amount: u64,
    // pub fees_paid: u64,
    // pub created_at: u64,
    // pub expires_at: Option<u64>,
    // pub settled_at: Option<u64>,
    pub metadata: Option<PayNotificationMetadata>,
}

#[derive(Deserialize, Debug)]
pub struct PayNotificationMetadata {
    // pub destination: String,
    pub tlv_records: Vec<TlvRecord>,
}

#[derive(Deserialize, Debug)]
pub struct TlvRecord {
    pub r#type: u64,
    pub value: String,
}

impl NWC {

    pub async fn new(uri: &str) -> Result<Self> {
        let uri = nip47::NostrWalletConnectURI::from_str(uri)
            .context(format!("Failed to parse NWC URI: {}", uri))?;

        let client = Client::default();
        client.add_relay(uri.relay_url.clone()).await
            .context(format!("Failed to add relay: {}", uri.relay_url))?;

        client.connect().await;
        println!("Connected to NWC relay {}", &uri.relay_url);

        Ok(Self {
            client,
            uri,
        })
    }

    pub async fn get_info(&self) -> Result<Option<GetInfoResult>> {
        let req = nip47::Request::get_info();
        let req_event = req.to_event(&self.uri)
            .context("Failed to create get_info event")?;

        let subscription = Filter::new()
            .author(self.uri.public_key)
            .kind(Kind::WalletConnectResponse)
            .event(req_event.id)
            .since(Timestamp::now());

        let _ = self.client.subscribe(vec![subscription], None).await
            .context("Failed to subscribe to NWC responses")?;

        self.client.send_event(req_event).await
            .context("Failed to send get_info event")?;

        let mut result: Option<GetInfoResult> = None;
        let mut notifications = self.client.notifications();

        while let Ok(notification) = notifications.recv().await {
            if let RelayPoolNotification::Event { event, .. } = notification {
                if event.kind == Kind::WalletConnectResponse {
                    let decrypt_res: String = nip04::decrypt(
                        &self.uri.secret,
                        &event.pubkey,
                        &event.content,
                    ).context("Failed to decrypt NWC response")?;
                    
                    let parsed: Value = serde_json::from_str(&decrypt_res).context("Failed to parse NWC response as JSON")?;

                    if let Some(inner_result) = parsed.get("result") {
                        result = Some(serde_json::from_value(inner_result.clone())
                            .context("Failed to parse GetInfoResult from JSON")?);
                    }
                }
            }

            break;
        }

        Ok(result)
    }

    pub async fn subscribe_boosts<F, Fut>(&self, timestamp: Timestamp, func: F) -> Result<()>
    where
        F: Fn(Boostagram) -> Fut,
        Fut: Future<Output = ()>,
    {
        let info = self.get_info().await
            .context("Failed to get NWC info")?
            .ok_or_else(|| anyhow::anyhow!("No info returned from NWC"))?;

        if info.notifications.contains(&"payment_received".to_string()) {
            println!("NWC listening for boosts");
            self.listen_for_boosts(func).await
                .context("Failed to listen for boosts")?;
        }
        else {
            println!("NWC polling for boosts");
            self.poll_boosts(timestamp, func).await
                .context("Failed to poll for boosts")?;
        }

        Ok(())
    }

    pub async fn listen_for_boosts<F, Fut>(&self, func: F) -> Result<()>
    where
        F: Fn(Boostagram) -> Fut,
        Fut: Future<Output = ()>,
    {
        let keys = Keys::new(self.uri.secret.clone());

        let subscription = Filter::new()
            .author(self.uri.public_key)
            .pubkey(keys.public_key())
            .kind(Kind::Custom(23196));

        let _ = self.client.subscribe(vec![subscription], None).await
            .context("Failed to subscribe to NWC notifications")?;

        let mut notifications = self.client.notifications();

        while let Ok(notification) = notifications.recv().await {
            if let RelayPoolNotification::Event { event, .. } = notification {
                if event.kind == Kind::WalletConnectResponse {
                    let decrypt_res: String = nip04::decrypt(
                        &self.uri.secret,
                        &event.pubkey,
                        &event.content,
                    ).context("Failed to decrypt NWC notification")?;
                    let parsed: Value = serde_json::from_str(&decrypt_res).context("Failed to parse NWC notification as JSON")?;
                    let notif_type = parsed.get("notification_type");

                    if let Some(notif_type) = notif_type {
                        if let Some("payment_received") = notif_type.as_str() {
                            if let Some(inner_result) = parsed.get("notification") {
                                let notification: PayNotification = serde_json::from_value(inner_result.clone())
                                    .context("Failed to parse PayNotification from JSON")?;

                                if let Some(meta) = notification.metadata {
                                    for tlv in meta.tlv_records {
                                        if tlv.r#type == 7629169 {
                                            if let Ok(bytes) = hex::decode(tlv.value) {
                                                if let Ok(boost) = serde_json::from_slice::<Boostagram>(&bytes) {
                                                    println!("boost: {:#?}", boost);
                                                    func(boost).await;
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn poll_boosts<F, Fut>(&self, timestamp: Timestamp, func: F) -> Result<()>
    where
        F: Fn(Boostagram) -> Fut,
        Fut: Future<Output = ()>,
    {
        let mut last_created_at = timestamp;

        loop {
            let params = nip47::ListTransactionsRequestParams {
                from: Some(last_created_at),
                until: None,
                limit: None,
                offset: None,
                unpaid: Some(false),
                transaction_type: Some(nip47::TransactionType::Incoming),
            };


            let nwc = NostrWC::new(self.uri.clone()); // Use `WebLNZapper::new().await` for WebLN

            let transactions = nwc.list_transactions(params).await;

            if let Err(err) = transactions {
                eprintln!("Error {:#?}", err);
            }
            else if let Ok(trans) = transactions {
                for tran in trans {
                    let created_at = tran.created_at;

                    if let Some(metadata) = tran.metadata {
                        if let Value::Array(tlvs) = &metadata["tlv_records"] {
                            for tlv in tlvs {
                                if let (Value::Number(tlv_type), Value::String(tlv_value)) = (&tlv["type"], &tlv["value"]) {
                                    if tlv_type.as_i64() == Some(7629169) {
                                        if let Ok(bytes) = hex::decode(tlv_value) {
                                            if let Ok(boost) = serde_json::from_slice::<Boostagram>(&bytes) {
                                                println!("boost: {:#?}", boost);
                                                func(boost).await;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if created_at > last_created_at {
                        last_created_at = created_at + 1;
                    }
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(5000)).await;
        }
    }

}