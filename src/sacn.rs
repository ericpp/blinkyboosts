use std::net::{IpAddr, SocketAddr};
use anyhow::Result;
use sacn::source::SacnSource;
use sacn::packet::ACN_SDT_MULTICAST_PORT;

pub struct Sacn {
    source: SacnSource,
    universe: u16,
    priority: u8,
}

impl Sacn {
    pub fn new(_broadcast_address: String, universe: Option<u16>) -> Result<Self> {
        let universe = universe.unwrap_or(1);

        // Create local address for the sACN source
        // Use a port offset from the multicast port to avoid conflicts
        // The sacn crate handles multicast/broadcast automatically
        let local_addr = SocketAddr::new(
            IpAddr::V4("0.0.0.0".parse().map_err(|e| anyhow::anyhow!("Failed to parse IP address: {}", e))?),
            ACN_SDT_MULTICAST_PORT + 1
        );

        // Create a new sACN source
        let mut source = SacnSource::with_ip("BlinkyBoosts", local_addr)
            .map_err(|e| anyhow::anyhow!("Failed to create sACN source: {}", e))?;

        // Register the universe
        source.register_universe(universe)
            .map_err(|e| anyhow::anyhow!("Failed to register universe {}: {}", universe, e))?;

        Ok(Self {
            source,
            universe,
            priority: 100, // Default priority
        })
    }

    pub fn send_dmx(&mut self, data: &[u8]) -> Result<()> {
        anyhow::ensure!(data.len() <= 513, "DMX data cannot exceed 513 bytes (including start code)");

        // Data should already include start code as first byte
        // If data doesn't start with 0, prepend start code
        let dmx_data = if data.is_empty() || data[0] != 0 {
            let mut with_start_code = vec![0u8; data.len() + 1];
            with_start_code[0] = 0; // Start code
            with_start_code[1..].copy_from_slice(data);
            with_start_code
        } else {
            data.to_vec()
        };

        // Send the DMX data to the universe
        // Using None for dst_ip means multicast, None for sync_uni means no synchronization delay
        self.source.send(&[self.universe], &dmx_data, Some(self.priority), None, None)
            .map_err(|e| anyhow::anyhow!("Failed to send sACN data: {}", e))?;

        Ok(())
    }

    pub fn trigger_for_sats(&mut self, sats: i64) -> Result<()> {
        let data = [
            sats.min(255).max(1) as u8,
            (sats % 256).max(1) as u8,
            ((sats / 256) % 256).max(1) as u8,
            ((sats / 65536) % 256).max(1) as u8,
        ];

        self.send_dmx(&data)
    }

    pub fn trigger_channel(&mut self, channel: u16, value: u8) -> Result<()> {
        anyhow::ensure!(channel > 0 && channel <= 512, "Channel must be between 1 and 512");

        let mut data = vec![0u8; channel as usize];
        data[(channel - 1) as usize] = value;

        self.send_dmx(&data)
    }

    pub fn trigger_toggle(toggle: &crate::config::Toggle, default_universe: u16, broadcast_address: String) -> Result<()> {
        let sacn_config = toggle.sacn.as_ref()
            .ok_or_else(|| anyhow::anyhow!("sACN toggle missing 'sacn' configuration"))?;

        let universe = sacn_config.universe.unwrap_or(default_universe);
        let mut sacn = Sacn::new(broadcast_address, Some(universe))?;
        sacn.trigger_channel(sacn_config.channel, sacn_config.value)
    }
}
