extern crate toml;

#[macro_use]
extern crate failure;
extern crate parking_lot;
extern crate rip;
extern crate tokio;

use failure::Error;

use std::cell::RefCell;
use std::env;
use std::fs::File;
use std::io::Read;
use std::sync::Arc;

use parking_lot::ReentrantMutex;

use tokio::runtime::Runtime;

use rip::RoutingTable;

fn main() -> Result<(), Error> {
    let config_file_path = env::args().nth(1).ok_or(format_err!("missing argument"))?;
    let mut config_file = File::open(config_file_path)?;
    let mut config_file_contents = Vec::new();
    config_file.read_to_end(&mut config_file_contents)?;

    let config = toml::from_slice(&config_file_contents)?;

    let table = RoutingTable::new(config)?;
    let thread_safe = Arc::new(ReentrantMutex::new(RefCell::new(table)));

    let future = RoutingTable::start(thread_safe);

    let mut rt = Runtime::new()?;
    rt.block_on(future)?;

    Ok(())
}
