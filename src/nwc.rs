use crate::boosts::Boostagram;
use crate::boostboard::BoostFilters;
use anyhow::{Context, Result};
use hex;
use nostr_sdk::{Client, Filter, Keys, Kind, NWC as NostrWC, RelayPoolNotification, Timestamp};
use nostr_sdk::nips::{nip04, nip47};
use serde::Deserialize;
use serde_json::Value;
use std::future::Future;
use std::str::FromStr;

#[derive(Clone)]
pub struct NWC {
    client: Client,
    uri: nip47::NostrWalletConnectURI,
    filters: BoostFilters,
}

#[derive(Deserialize, Debug)]
pub struct GetInfoResult {
    pub notifications: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct PayNotification {
    pub metadata: Option<PayNotificationMetadata>,
}

#[derive(Deserialize, Debug)]
pub struct PayNotificationMetadata {
    pub tlv_records: Vec<TlvRecord>,
}

#[derive(Deserialize, Debug)]
pub struct TlvRecord {
    pub r#type: u64,
    pub value: String,
}

const BOOST_TLV_TYPE: u64 = 7629169;
const POLL_INTERVAL_MS: u64 = 5000;

impl NWC {
    pub async fn new(uri: &str, filters: BoostFilters) -> Result<Self> {
        let uri = nip47::NostrWalletConnectURI::from_str(uri)
            .context("Failed to parse NWC URI")?;

        let client = Client::default();
        client.add_relay(uri.relay_url.clone()).await
            .context("Failed to add relay")?;

        client.connect().await;
        println!("Connected to NWC relay {}", &uri.relay_url);

        Ok(Self { client, uri, filters })
    }

    pub async fn get_info(&self) -> Result<Option<GetInfoResult>> {
        let req = nip47::Request::get_info();
        let req_event = req.to_event(&self.uri)?;

        let subscription = Filter::new()
            .author(self.uri.public_key)
            .kind(Kind::WalletConnectResponse)
            .event(req_event.id)
            .since(Timestamp::now());

        self.client.subscribe(vec![subscription], None).await?;
        self.client.send_event(req_event).await?;

        let mut notifications = self.client.notifications();
        while let Ok(notification) = notifications.recv().await {
            if let RelayPoolNotification::Event { event, .. } = notification {
                if event.kind == Kind::WalletConnectResponse {
                    let decrypted = nip04::decrypt(&self.uri.secret, &event.pubkey, &event.content)?;
                    let parsed: Value = serde_json::from_str(&decrypted)?;

                    if let Some(result) = parsed.get("result") {
                        return Ok(Some(serde_json::from_value(result.clone())?));
                    }
                }
            }
            break;
        }

        Ok(None)
    }

    pub async fn subscribe_boosts<F, Fut>(&self, timestamp: Timestamp, func: F) -> Result<()>
    where
        F: Fn(Boostagram) -> Fut,
        Fut: Future<Output = ()>,
    {
        let info = self.get_info().await?
            .ok_or_else(|| anyhow::anyhow!("No info returned from NWC"))?;

        if info.notifications.contains(&"payment_received".to_string()) {
            println!("NWC listening for boosts");
            self.listen_for_boosts(func).await
        } else {
            println!("NWC polling for boosts");
            self.poll_boosts(timestamp, func).await
        }
    }

    async fn listen_for_boosts<F, Fut>(&self, func: F) -> Result<()>
    where
        F: Fn(Boostagram) -> Fut,
        Fut: Future<Output = ()>,
    {
        let keys = Keys::new(self.uri.secret.clone());
        let subscription = Filter::new()
            .author(self.uri.public_key)
            .pubkey(keys.public_key())
            .kind(Kind::Custom(23196));

        self.client.subscribe(vec![subscription], None).await?;
        let mut notifications = self.client.notifications();

        while let Ok(notification) = notifications.recv().await {
            if let RelayPoolNotification::Event { event, .. } = notification {
                if event.kind == Kind::WalletConnectResponse {
                    if let Some(boost) = self.extract_boost_from_notification(&event).await? {
                        let event_ts = event.created_at.as_u64() as i64;
                        if self.filters.matches_timestamp(event_ts) && self.filters.matches_boost(&boost) {
                            println!("boost: {:#?}", boost);
                            func(boost).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn extract_boost_from_notification(&self, event: &nostr_sdk::Event) -> Result<Option<Boostagram>> {
        let decrypted = nip04::decrypt(&self.uri.secret, &event.pubkey, &event.content)?;
        let parsed: Value = serde_json::from_str(&decrypted)?;

        if parsed.get("notification_type").and_then(|v| v.as_str()) == Some("payment_received") {
            if let Some(notification) = parsed.get("notification") {
                let pay_notif: PayNotification = serde_json::from_value(notification.clone())?;

                if let Some(meta) = pay_notif.metadata {
                    for tlv in meta.tlv_records {
                        if tlv.r#type == BOOST_TLV_TYPE {
                            if let Ok(bytes) = hex::decode(tlv.value) {
                                if let Ok(boost) = serde_json::from_slice::<Boostagram>(&bytes) {
                                    return Ok(Some(boost));
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    async fn poll_boosts<F, Fut>(&self, timestamp: Timestamp, func: F) -> Result<()>
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

            let nwc = NostrWC::new(self.uri.clone());

            match nwc.list_transactions(params).await {
                Ok(transactions) => {
                    for tran in transactions {
                        if let Some(boost) = self.extract_boost_from_transaction(&tran) {
                            let created_at_ts = tran.created_at.as_u64() as i64;
                            if self.filters.matches_timestamp(created_at_ts) && self.filters.matches_boost(&boost) {
                                println!("boost: {:#?}", boost);
                                func(boost).await;
                            }
                        }

                        if tran.created_at > last_created_at {
                            last_created_at = tran.created_at + 1;
                        }
                    }
                }
                Err(err) => eprintln!("Error polling transactions: {:#?}", err),
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    fn extract_boost_from_transaction(&self, tran: &nip47::LookupInvoiceResponseResult) -> Option<Boostagram> {
        let metadata = tran.metadata.as_ref()?;
        let tlvs = metadata.get("tlv_records")?.as_array()?;

        for tlv in tlvs {
            let tlv_type = tlv.get("type")?.as_i64()?;
            let tlv_value = tlv.get("value")?.as_str()?;

            if tlv_type == BOOST_TLV_TYPE as i64 {
                let bytes = hex::decode(tlv_value).ok()?;
                return serde_json::from_slice::<Boostagram>(&bytes).ok();
            }
        }

        None
    }

    pub async fn load_previous_boosts<F, Fut>(&self, from: Option<Timestamp>, mut callback: F) -> Result<Option<Timestamp>>
    where
        F: FnMut(Boostagram) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        let nwc = NostrWC::new(self.uri.clone());

        // Use the maximum of from and filters.after to start from the earliest relevant timestamp
        let from_timestamp = match (from, self.filters.after) {
            (Some(f), Some(a)) => f.max(a),
            (Some(f), None) => f,
            (None, Some(a)) => a,
            (None, None) => Timestamp::from_secs(0),
        };

        let until_timestamp = self.filters.before;

        let params = nip47::ListTransactionsRequestParams {
            from: Some(from_timestamp),
            until: until_timestamp,
            limit: None,
            offset: None,
            unpaid: Some(false),
            transaction_type: Some(nip47::TransactionType::Incoming),
        };

        let mut last_boost_at = from;

        match nwc.list_transactions(params).await {
            Ok(transactions) => {
                // Sort transactions by created_at to process in chronological order
                let mut sorted_transactions: Vec<_> = transactions.into_iter().collect();
                sorted_transactions.sort_by_key(|t| t.created_at);

                for tran in sorted_transactions {
                    if let Some(boost) = self.extract_boost_from_transaction(&tran) {
                        let created_at_ts = tran.created_at.as_u64() as i64;
                        if self.filters.matches_timestamp(created_at_ts) && self.filters.matches_boost(&boost) {
                            callback(boost).await;

                            if last_boost_at.map_or(true, |last| tran.created_at > last) {
                                last_boost_at = Some(tran.created_at);
                            }
                        }
                    }
                }
            }
            Err(err) => {
                eprintln!("Error loading previous transactions from NWC: {:#?}", err);
                return Err(anyhow::anyhow!("Failed to load previous transactions: {:#}", err));
            }
        }

        Ok(last_boost_at)
    }
}