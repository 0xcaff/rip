use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

#[derive(Deserialize)]
pub struct Config {
    pub bind_address: SocketAddr,
    pub neighbors: Vec<Neighbor>,
}

#[derive(Deserialize, Clone)]
pub struct Neighbor {
    pub address: SocketAddrV4,
    pub subnet_mask: Ipv4Addr,
    pub metric: u32,
}

impl Neighbor {
    pub fn network_prefix(&self) -> u32 {
        u32::from(*self.address.ip()) & u32::from(self.subnet_mask)
    }
}
