use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use rosc::{OscMessage, OscPacket, OscType, encoder};
use anyhow::{Context, Result};

pub struct Osc {
    sock: UdpSocket,
    to_addr: SocketAddrV4,
}

impl Osc {
    pub fn new(address: &str) -> Result<Self> {
        let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
            .context("Unable to bind to host address")?;

        sock.set_broadcast(true)
            .context("Unable to enable broadcast")?;

        let to_addr = address.parse()
            .with_context(|| format!("Unable to parse OSC address: {}", address))?;

        Ok(Self { sock, to_addr })
    }

    pub fn trigger_path(&self, path: &str, args: Vec<OscType>) -> Result<()> {
        println!("Triggering OSC path with args: {} {:?}", path, args);

        let msg_buf = encoder::encode(&OscPacket::Message(OscMessage {
            addr: path.to_string(),
            args,
        }))
        .with_context(|| format!("Failed to encode OSC message for path: {}", path))?;

        self.sock.send_to(&msg_buf, self.to_addr)
            .with_context(|| format!("Failed to send OSC message to {}", self.to_addr))?;

        Ok(())
    }
}
