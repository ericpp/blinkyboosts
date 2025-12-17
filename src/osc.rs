use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use rosc::{OscMessage, OscPacket, OscType};
use rosc::encoder;
use anyhow::{Context, Result, anyhow};

pub struct Osc {
    sock: UdpSocket,
    to_addr: SocketAddrV4,
}

impl Osc {
    pub fn new(address: String) -> Result<Self> {
        // Bind to all interfaces (0.0.0.0) to allow sending to any network interface
        let host_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        let sock = UdpSocket::bind(host_addr)
            .context("Unable to bind to host address")?;
        
        // Enable broadcast in case it's needed
        sock.set_broadcast(true)
            .context("Unable to enable broadcast")?;

        let to_addr = address.parse::<SocketAddrV4>()
            .context(format!("Unable to parse OSC address: {}", address))?;

        Ok(Self {
            sock,
            to_addr,
        })
    }

    pub fn trigger_path(&self, path: String) -> Result<()> {
        println!("Triggering OSC path: {}", path);

        let msg_buf = encoder::encode(&OscPacket::Message(OscMessage {
            addr: path.clone(),
            args: vec![OscType::Int(255)],
        }))
        .context(format!("Failed to encode OSC message for path: {}", path))?;

        self.sock.send_to(&msg_buf, self.to_addr)
            .context(format!("Failed to send OSC message to {}", self.to_addr))?;

        Ok(())
    }

    pub fn trigger_for_sats(&self, sats: i64) -> Result<()> {
        self.trigger_path("/boost".to_string())
            .context("Failed to trigger base boost path")?;
            
        self.trigger_path(format!("/boost/{}", sats))
            .context(format!("Failed to trigger boost path for {} sats", sats))?;

        let sats_str = sats.to_string();
        let endswith = sats_str.chars().last()
            .ok_or_else(|| anyhow!("Sats value has no digits"))?;

        self.trigger_path(format!("/boost/endswith/{}", endswith))
            .context(format!("Failed to trigger endswith path for digit {}", endswith))?;

        Ok(())
    }
}