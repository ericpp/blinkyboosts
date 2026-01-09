use nostr_sdk::{Timestamp, Client, Options, Filter, PublicKey, Kind, SubscriptionId, RelayPoolNotification};
use crate::boosts::Boostagram;
use nostr_sdk::prelude::Output;
use serde::{Serialize, Deserialize};
use anyhow::{Context, Result};
use std::future::Future;

#[derive(Serialize, Deserialize, Debug)]
struct BoostBoardEvent {
    boostagram: Option<StoredBoostagram>,
}

#[derive(Clone, Debug)]
pub struct BoostFilters {
    pub podcasts: Option<Vec<String>>,
    pub episode_guids: Option<Vec<String>>,
    pub event_guids: Option<Vec<String>>,
    pub before: Option<Timestamp>,
    pub after: Option<Timestamp>,
}

impl BoostFilters {
    fn has_content_filters(&self) -> bool {
        self.podcasts.is_some() || self.episode_guids.is_some() || self.event_guids.is_some()
    }

    pub fn matches_boost(&self, boost: &Boostagram) -> bool {
        if !self.has_content_filters() {
            return true;
        }

        let podcast_match = self.podcasts.as_ref()
            .map_or(false, |ps| ps.iter().any(|p| boost.podcast.to_lowercase().contains(&p.to_lowercase())));

        let episode_match = self.episode_guids.as_ref()
            .map_or(false, |guids| !boost.episode_guid.is_empty() && guids.contains(&boost.episode_guid));

        let event_match = self.event_guids.as_ref()
            .map_or(false, |guids| !boost.event_guid.is_empty() && guids.contains(&boost.event_guid));

        podcast_match || episode_match || event_match
    }

    pub fn matches_timestamp(&self, ts: i64) -> bool {
        self.after.map_or(true, |a| ts > a.as_u64() as i64)
            && self.before.map_or(true, |b| ts < b.as_u64() as i64)
    }

    fn get_since_timestamp(&self, since: Option<Timestamp>) -> Timestamp {
        since.max(self.after).unwrap_or_else(|| Timestamp::from_secs(0))
    }
}

#[derive(Clone)]
pub struct BoostBoard {
    client: Client,
    pubkey: PublicKey,
    filters: BoostFilters,
}

impl BoostBoard {
    pub async fn new(relay_addrs: &[String], pubkey: &str, filters: BoostFilters) -> Result<Self> {
        let client = Client::builder()
            .opts(Options::new().wait_for_send(false))
            .build();

        for addr in relay_addrs {
            client.add_relay(addr).await
                .context(format!("Failed to add relay: {}", addr))?;
        }
        client.connect().await;

        let pubkey = PublicKey::from_hex(pubkey)
            .context(format!("Failed to parse pubkey: {}", pubkey))?;

        Ok(Self { client, pubkey, filters })
    }

    pub async fn subscribe(&self, since: Option<Timestamp>) -> Result<SubscriptionId> {
        let mut filter = Filter::new()
            .author(self.pubkey)
            .kind(Kind::ApplicationSpecificData)
            .since(self.filters.get_since_timestamp(since));

        if let Some(before) = self.filters.before {
            filter = filter.until(before);
        }
println!("Boostboard subscribe filters: {:#?}", filter);
        let Output { val: sub_id, .. } = self.client
            .subscribe(vec![filter], None)
            .await
            .context("Failed to subscribe to boostboard")?;

        Ok(sub_id)
    }

    pub async fn handle_boosts<F, Fut>(&self, sub_id: SubscriptionId, func: F) -> Result<()>
    where
        F: Fn(Boostagram, Timestamp) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let filters = self.filters.clone();

        self.client.handle_notifications(move |notification| {
            let filters = filters.clone();
            let sub_id_check = sub_id.clone();
            let func = func.clone();

            async move {
                if let RelayPoolNotification::Event { subscription_id, event, .. } = notification {
                    if subscription_id != sub_id_check || !filters.matches_timestamp(event.created_at.as_u64() as i64) {
                        println!("Timestamp not matched: {:#?}", event);
                        return Ok(false);
                    }


                    match serde_json::from_str::<StoredBoostInfo>(&event.content) {
                        Ok(info) => {
                            match info.to_boostagram() {
                                Some(boost) => {
                                    if filters.matches_boost(&boost) {
                                        println!("Live boost: {:#?}", boost);
                                        func(boost, event.created_at).await;
                                    } else {
                                        println!("Boost doesn't match filters: {:#?}", boost);
                                    }
                                }
                                None => {
                                    println!("Event has no boost info: {:#?}", info);
                                }
                            }
                        }
                        Err(e) => {
                            println!("Error parsing boost event: {:#?}", e);
                        }
                    }
                }
                Ok(false)
            }
        })
        .await
        .context("Failed to handle boostboard notifications")?;

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct StoredBoostInfo {
    identifier: String,
    creation_date: i64,
    boostagram: Option<StoredBoostagram>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StoredBoostagram {
    pub action: Option<String>,
    pub app_name: Option<String>,
    #[serde(rename = "blockGuid")]
    pub block_guid: Option<String>,
    #[serde(rename = "eventApi")]
    pub event_api: Option<String>,
    #[serde(rename = "eventGuid")]
    pub event_guid: Option<String>,
    pub boost_link: Option<String>,
    pub episode: Option<String>,
    pub episode_guid: Option<String>,
    pub guid: Option<String>,
    pub message: Option<String>,
    pub name: Option<String>,
    pub podcast: Option<String>,
    #[serde(rename = "remoteFeedGuid")]
    pub remote_feed_guid: Option<String>,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub ts: Option<i64>,
    pub value_msat_total: Option<i64>,
}

impl StoredBoostInfo {
    pub fn to_boostagram(&self) -> Option<Boostagram> {
        if self.boostagram.is_none() {
            return None;
        }

        let boost = self.boostagram.as_ref().unwrap();

        Some(Boostagram {
            boost_type: "stored_boost".to_string(),
            action: boost.action.clone().unwrap_or_default(),
            identifier: self.identifier.clone(),
            creation_date: self.creation_date,
            sender_name: boost.sender_name.clone().unwrap_or_default(),
            app_name: boost.app_name.clone().unwrap_or_default(),
            podcast: boost.podcast.clone().unwrap_or_default(),
            episode: boost.episode.clone().unwrap_or_default(),
            sats: boost.value_msat_total.clone().unwrap_or_default() / 1000,
            message: boost.message.clone().unwrap_or_default(),
            event_guid: boost.event_guid.clone().unwrap_or_default(),
            episode_guid: boost.episode_guid.clone().unwrap_or_default(),
            remote_feed: None,
            remote_item: None,
            is_old: true,
        })
    }
}


pub struct StoredBoosts {
    filters: BoostFilters,
}

impl StoredBoosts {
    pub fn new(filters: BoostFilters) -> Self {
        Self { filters }
    }

    pub async fn load<F, Fut>(&self, mut callback: F) -> Result<Option<Timestamp>>
    where
        F: FnMut(Boostagram) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        let mut page = 1;
        let mut last_boost_at = self.filters.after;

        loop {
            let boosts = self.fetch_page(page).await?;
            if boosts.is_empty() {
                break;
            }

            last_boost_at = self.update_last_boost_timestamp(last_boost_at, &boosts);
            self.process_boosts(boosts, &mut callback).await;
            page += 1;
        }

        Ok(last_boost_at)
    }

    fn update_last_boost_timestamp(&self, current: Option<Timestamp>, boosts: &[StoredBoostInfo]) -> Option<Timestamp> {
        boosts.iter()
            .map(|b| Timestamp::from_secs(b.creation_date as u64))
            .max()
            .map(|new_ts| current.map_or(new_ts, |old| if new_ts > old { new_ts } else { old }))
            .or(current)
    }

    async fn process_boosts<F, Fut>(&self, mut boosts: Vec<StoredBoostInfo>, callback: &mut F)
    where
        F: FnMut(Boostagram) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        boosts.sort_by_key(|b| b.creation_date);

        for invoice in boosts {
            if let Some(boost) = invoice.to_boostagram() {
                if self.filters.matches_timestamp(invoice.creation_date) && self.filters.matches_boost(&boost) {
                    callback(boost).await;
                } else {
                    println!("Stored boost doesn't match filters: {:#?}", boost);
                }
            }
        }
    }

    async fn fetch_page(&self, page: u32) -> Result<Vec<StoredBoostInfo>> {
        let url = self.build_url(page)?;
        println!("StoredBoosts url: {:#?}", url);
        let response = reqwest::get(url).await
            .context("Failed to fetch boosts from API")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("API error: {}", response.status()));
        }

        response.json().await.context("Failed to parse API response")
    }

    fn build_url(&self, page: u32) -> Result<reqwest::Url> {
        let mut url = reqwest::Url::parse("https://boostboard.vercel.app/api/boosts")
            .context("Failed to parse base URL")?;

        {
            let mut query = url.query_pairs_mut();
            query.append_pair("page", &page.to_string());
            query.append_pair("items", "1000");

            if let Some(ref podcasts) = self.filters.podcasts {
                query.append_pair("podcast", &podcasts.join(","));
            }
            if let Some(ref guids) = self.filters.episode_guids {
                query.append_pair("episodeGuid", &guids.join(","));
            }
            if let Some(ref guids) = self.filters.event_guids {
                query.append_pair("eventGuid", &guids.join(","));
            }
            if let Some(ts) = self.filters.before {
                query.append_pair("created_at_lt", &ts.as_u64().to_string());
            }
            if let Some(ts) = self.filters.after {
                query.append_pair("created_at_gt", &ts.as_u64().to_string());
            }
        }

        Ok(url)
    }
}