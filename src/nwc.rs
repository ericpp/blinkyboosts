use crate::boosts::Boostagram;

use hex;

use nostr_sdk::nips::nip47::ListTransactionsRequestParams;
use nostr_sdk::nips::nip47::TransactionType;
use nostr_sdk::NWC as NostrWC;
use nostr_sdk::prelude::NostrWalletConnectURI;
use nostr_sdk::Timestamp;

use serde_json::Value;

use std::error::Error;
use std::future::Future;
use std::str::FromStr;

#[derive(Clone)]
pub struct NWC {
    pub nwc: NostrWC,
}

impl NWC {

    pub fn new(uri: &str) -> Result<Self, Box<dyn Error>> {
        let connect_uri = NostrWalletConnectURI::from_str(uri)?;
        let nwc = NostrWC::new(connect_uri); // Use `WebLNZapper::new().await` for WebLN

        Ok(Self {
            nwc
        })
    }

    pub async fn subscribe_boosts<F, Fut>(&self, timestamp: Timestamp, func: F) -> Result<(), Box<dyn Error>>
    where
        F: Fn(Boostagram) -> Fut,
        Fut: Future<Output = ()>,
    {
        let mut last_created_at = timestamp;

        loop {
            let params = ListTransactionsRequestParams {
                from: Some(last_created_at),
                until: None,
                limit: None,
                offset: None,
                unpaid: Some(false),
                transaction_type: Some(TransactionType::Incoming),
            };

            let transactions = self.nwc.list_transactions(params).await;

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