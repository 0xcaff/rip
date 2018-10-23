use failure::Error;
use proto::ALLOWED_NETMASK;
use std::net::{SocketAddr, SocketAddrV4};

#[derive(Deserialize)]
pub struct Config {
    pub bind_address: SocketAddr,
    pub neighbors: Vec<Neighbor>,
}

#[derive(Deserialize)]
pub struct Neighbor {
    pub ip_address: SocketAddrV4,
    pub metric: u32,
}

impl Neighbor {
    pub fn network_prefix(&self) -> u32 {
        u32::from(*self.ip_address.ip()) & ALLOWED_NETMASK
    }
}
