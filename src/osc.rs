use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use rosc::{OscMessage, OscPacket, OscType};
use rosc::encoder;
use std::error::Error;

pub struct Osc {
    sock: UdpSocket,
    to_addr: SocketAddrV4,
}

impl Osc {
    pub fn new(address: String) -> Self {
        let host_addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);
        let sock = UdpSocket::bind(host_addr).expect("Unable to bind to host address");
        let to_addr = address.parse::<SocketAddrV4>();

        if let Err(err) = &to_addr {
            eprintln!("Unable to parse OSC address {}: {}", address, err);
        }

        let to_addr = to_addr.unwrap();

        Self {
            sock,
            to_addr,
        }
    }

    pub fn trigger_path(&self, path: String) -> Result<(), Box<dyn Error>> {
        println!("Triggering OSC path: {}", path);

        let msg_buf = encoder::encode(&OscPacket::Message(OscMessage {
            addr: path,
            args: vec![OscType::Int(255)],
        }))?;

        self.sock.send_to(&msg_buf, self.to_addr)?;

        Ok(())
    }

    pub fn trigger_for_sats(&self, sats: i64) -> Result<(), Box<dyn Error>> {
        self.trigger_path("/boost".to_string())?;
        self.trigger_path(format!("/boost/{}", sats))?;

        let sats_str = sats.to_string();
        let endswith = &sats_str.chars().last().unwrap();

        self.trigger_path(format!("/boost/endswith/{}", endswith))?;

        Ok(())
    }
}