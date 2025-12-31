use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Boostagram {
	pub boost_type: String,
	pub action: String,
	pub identifier: String,
	pub creation_date: i64,
	pub sender_name: String,
	pub app_name: String,
	pub podcast: String,
	pub episode: String,
	pub sats: i64,
	pub message: String,

	pub event_guid: String,
	pub episode_guid: String,

	pub remote_feed: Option<String>,
	pub remote_item: Option<String>,

	pub is_old: bool,
}