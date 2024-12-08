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

use std::error::Error;
use std::future::Future;
use std::str::FromStr;

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

    pub async fn new(uri: &str) -> Result<Self, Box<dyn Error>> {
        let uri = nip47::NostrWalletConnectURI::from_str(uri)?;

        let client = Client::default();
        client.add_relay(uri.relay_url.clone()).await?;

        client.connect().await;
        println!("Connected to NWC relay {}", &uri.relay_url);

        Ok(Self {
            client,
            uri,
        })
    }

    pub async fn get_info(&self) -> Result<Option<GetInfoResult>, Box<dyn Error>> {
        let req = nip47::Request::get_info();
        let req_event = req.to_event(&self.uri).unwrap();

        let subscription = Filter::new()
            .author(self.uri.public_key)
            .kind(Kind::WalletConnectResponse)
            .event(req_event.id)
            .since(Timestamp::now());

        let _ = self.client.subscribe(vec![subscription], None).await;

        self.client.send_event(req_event).await.unwrap();

        let mut result: Option<GetInfoResult> = None;
        let mut notifications = self.client.notifications();

        while let Ok(notification) = notifications.recv().await {
            if let RelayPoolNotification::Event { event, .. } = notification {
                let decrypt_res: String = nip04::decrypt(&self.uri.secret, &event.author(), event.content())?;
                let parsed: Value = serde_json::from_str(&decrypt_res)?;

                if let Some(inner_result) = parsed.get("result") {
                    result = Some(serde_json::from_value(inner_result.clone())?);
                }
            }

            break;
        }

        Ok(result)
    }

    pub async fn subscribe_boosts<F, Fut>(&self, timestamp: Timestamp, func: F) -> Result<(), Box<dyn Error>>
    where
        F: Fn(Boostagram) -> Fut,
        Fut: Future<Output = ()>,
    {
        let info = self.get_info().await?.unwrap();

        if info.notifications.contains(&"payment_received".to_string()) {
            println!("NWC listening for boosts");
            let _ = self.listen_for_boosts(func).await;
        }
        else {
            println!("NWC polling for boosts");
            let _ = self.poll_boosts(timestamp, func).await;
        }

        Ok(())
    }

    pub async fn listen_for_boosts<F, Fut>(&self, func: F) -> Result<(), Box<dyn Error>>
    where
        F: Fn(Boostagram) -> Fut,
        Fut: Future<Output = ()>,
    {
        let keys = Keys::new(self.uri.secret.clone());

        let subscription = Filter::new()
            .author(self.uri.public_key)
            .pubkey(keys.public_key())
            .kind(Kind::Custom(23196));

        let _ = self.client.subscribe(vec![subscription], None).await;

        let mut notifications = self.client.notifications();

        while let Ok(notification) = notifications.recv().await {
            if let RelayPoolNotification::Event { event, .. } = notification {
                let decrypt_res: String = nip04::decrypt(&self.uri.secret, &event.author(), event.content())?;
                let parsed: Value = serde_json::from_str(&decrypt_res)?;
                let notif_type = parsed.get("notification_type");

                if let Some(notif_type) = notif_type {
                    if let Some("payment_received") = notif_type.as_str() {
                        if let Some(inner_result) = parsed.get("notification") {
                            let notification: PayNotification = serde_json::from_value(inner_result.clone())?;

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

        Ok(())
    }

    pub async fn poll_boosts<F, Fut>(&self, timestamp: Timestamp, func: F) -> Result<(), Box<dyn Error>>
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