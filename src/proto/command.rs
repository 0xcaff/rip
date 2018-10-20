use failure::Error;

#[derive(Eq, PartialEq, Debug)]
pub enum Command {
    Request,
    Response,
}

impl Command {
    pub fn from(code: u8) -> Result<Command, Error> {
        let variant = match code {
            1 => Command::Request,
            2 => Command::Response,
            _ => return Err(format_err!("Unknown command code {}", code)),
        };

        Ok(variant)
    }

    pub fn as_code(&self) -> u8 {
        match self {
            Command::Request => 1,
            Command::Response => 2,
        }
    }
}
