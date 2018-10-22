#[macro_use]
extern crate nom;

#[macro_use]
extern crate failure;

extern crate byteorder;

#[macro_use]
extern crate serde_derive;
extern crate serde;

extern crate tokio;

extern crate parking_lot;

pub type NetworkPrefix = u32;

mod proto;
mod table;
