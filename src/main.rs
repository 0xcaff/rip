extern crate toml;

#[macro_use]
extern crate failure;
extern crate rip;
extern crate tokio;

use failure::Error;

use std::env;
use std::fs::File;
use std::io::Read;

use rip::start;
use tokio::runtime::Runtime;

fn main() -> Result<(), Error> {
    let config_file_path = env::args().nth(1).ok_or(format_err!("missing argument"))?;
    let mut config_file = File::open(config_file_path)?;
    let mut config_file_contents = Vec::new();
    config_file.read_to_end(&mut config_file_contents)?;

    let config = toml::from_slice(&config_file_contents)?;

    let mut rt = Runtime::new()?;
    rt.block_on(start(config))?;

    Ok(())
}
