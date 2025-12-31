use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use anyhow::Result;
use artnet_protocol::*;

pub struct ArtNet {
    sock: UdpSocket,
    to_addr: SocketAddrV4,
    universe: u16,
}

impl ArtNet {
    pub fn new(broadcast_address: String, local_address: Option<String>, universe: Option<u16>) -> Result<Self> {
        // Bind to specific local interface if provided, otherwise bind to all interfaces
        let bind_addr = if let Some(local_addr) = local_address {
            local_addr.parse::<Ipv4Addr>()
                .map_err(|e| anyhow::anyhow!("Invalid local address '{}': {}", local_addr, e))?
        } else {
            Ipv4Addr::UNSPECIFIED
        };

        let sock = UdpSocket::bind(SocketAddrV4::new(bind_addr, 0))?;

        // Enable broadcast to allow sending to broadcast addresses
        sock.set_broadcast(true)?;

        let to_addr = if broadcast_address.contains(':') {
            broadcast_address.parse()?
        } else {
            format!("{}:6454", broadcast_address).parse()?
        };

        Ok(Self {
            sock,
            to_addr,
            universe: universe.unwrap_or(0),
        })
    }

    pub fn send_dmx(&self, data: &[u8]) -> Result<()> {
        anyhow::ensure!(data.len() <= 512, "DMX data cannot exceed 512 bytes");

        let output = Output {
            data: data.to_vec().into(),
            port_address: PortAddress::try_from(self.universe)?,
            ..Output::default()
        };

        let packet = ArtCommand::Output(output).write_to_buffer()?;
        self.sock.send_to(&packet, self.to_addr)
            .map_err(|e| anyhow::anyhow!("Failed to send Art-Net packet to {}: {}. Make sure the broadcast address matches your network interface.", self.to_addr, e))?;
        Ok(())
    }

    pub fn trigger_for_sats(&self, sats: i64) -> Result<()> {
        let data = [
            0, // Start code
            sats.min(255).max(1) as u8,
            (sats % 256).max(1) as u8,
            ((sats / 256) % 256).max(1) as u8,
            ((sats / 65536) % 256).max(1) as u8,
        ];

        self.send_dmx(&data)
    }

    pub fn trigger_channel(&self, channel: u16, value: u8) -> Result<()> {
        anyhow::ensure!(channel > 0 && channel <= 512, "Channel must be between 1 and 512");

        let mut data = vec![0u8; channel as usize];
        data[(channel - 1) as usize] = value;

        self.send_dmx(&data)
    }

    pub fn trigger_toggle(toggle: &crate::config::Toggle, default_universe: u16, broadcast_address: String, local_address: Option<String>) -> Result<()> {
        let artnet_config = toggle.artnet.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Art-Net toggle missing 'artnet' configuration"))?;

        let universe = artnet_config.universe.unwrap_or(default_universe);
        let artnet = ArtNet::new(broadcast_address, local_address, Some(universe))?;
        artnet.trigger_channel(artnet_config.channel, artnet_config.value)
    }
}
