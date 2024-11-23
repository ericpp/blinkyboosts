use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Boostagram {
	pub podcast:          String,
	pub action:           String,
	pub sender_name:      Option<String>,
	pub app_name:         String,
	pub message:          Option<String>,
	pub value_msat_total: i64,
}