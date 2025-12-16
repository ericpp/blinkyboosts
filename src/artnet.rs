use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use anyhow::Result;
use artnet_protocol::*;

pub struct ArtNet {
    sock: UdpSocket,
    to_addr: SocketAddrV4,
    universe: u16,
}

impl ArtNet {
    pub fn new(broadcast_address: String, universe: Option<u16>) -> Result<Self> {
        // Bind to 0.0.0.0 (all interfaces) instead of localhost to allow sending to different networks
        let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))?;
        
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

    pub fn send_custom(&self, channels: &[(usize, u8)]) -> Result<()> {
        let mut data = vec![0u8; 512];
        
        for &(ch, val) in channels {
            if ch > 0 && ch < 512 {
                data[ch] = val;
            }
        }
        
        let len = data.iter().rposition(|&x| x != 0).unwrap_or(0) + 1;
        self.send_dmx(&data[..len])
    }
}