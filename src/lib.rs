#[macro_use]
extern crate nom;

#[macro_use]
extern crate failure;

extern crate byteorder;

#[macro_use]
extern crate serde_derive;
extern crate serde;

extern crate tokio;

#[macro_use]
extern crate futures;

extern crate parking_lot;

pub type NetworkPrefix = u32;

mod config;
mod udp;
mod proto;
mod table;

pub use config::Config;
pub use table::RoutingTable;
