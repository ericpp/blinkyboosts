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

    pub fn send_custom(&mut self, channels: &[(usize, u8)]) -> Result<()> {
        let mut data = vec![0u8; 512];
        
        // DMX channels are 1-indexed, so channel 1 goes to index 0 in our data array
        // (send_dmx will add the start code at the beginning)
        for &(ch, val) in channels {
            if ch > 0 && ch <= 512 {
                data[ch - 1] = val;
            }
        }
        
        let len = data.iter().rposition(|&x| x != 0).map(|i| i + 1).unwrap_or(0);
        if len > 0 {
            self.send_dmx(&data[..len])
        } else {
            Ok(())
        }
    }
}
