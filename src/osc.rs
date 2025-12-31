use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use rosc::{OscMessage, OscPacket, OscType, encoder};
use anyhow::{Context, Result, anyhow};

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

    pub fn trigger_for_sats(&self, sats: i64) -> Result<()> {
        // Send the sats value as an integer to the /boost path
        self.trigger_path("/boost", vec![OscType::Int(sats as i32)])
    }

    pub fn trigger_toggle(&self, toggle: &crate::config::Toggle) -> Result<()> {
        let osc_config = toggle.osc.as_ref()
            .ok_or_else(|| anyhow!("OSC toggle missing 'osc' configuration"))?;

        let arg = match &osc_config.arg_value {
            crate::config::OscArgValue::String(s) => OscType::String(s.clone()),
            crate::config::OscArgValue::Int(i) => OscType::Int(*i as i32),
            crate::config::OscArgValue::Float(f) => OscType::Float(*f as f32),
        };

        self.trigger_path(&osc_config.path, vec![arg])
    }
}
