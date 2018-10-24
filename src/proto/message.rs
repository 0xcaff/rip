use super::entry::{parse_entry, Entry};
use proto::command::Command;

use byteorder::{NetworkEndian, WriteBytesExt};
use failure::Error;
use nom::be_u8;
use std::io::Write;

named!(pub parse_message<Message>, do_parse!(
   command: be_u8               >>
            tag!([2])           >>
            tag!([0, 0])        >>
   entries: many0!(complete!(parse_entry)) >>

   (Message {
       command: Command::from(command).unwrap(),
       entries,
   })
));

#[derive(Eq, PartialEq, Debug)]
pub struct Message {
    pub command: Command,
    pub entries: Vec<Entry>,
}

impl Message {
    pub fn decode(raw: &[u8]) -> Result<Message, Error> {
        let (_, message) =
            parse_message(raw).map_err(|err| format_err!("Failed to parse message: {}", err))?;

        Ok(message)
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut output = Vec::new();

        output.write_u8(self.command.as_code())?;
        output.write_u8(2)?;
        output.write_u16::<NetworkEndian>(0)?;

        for entry in &self.entries {
            let encoded_entry = entry.encode()?;
            output.write(&encoded_entry)?;
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Entry, Message};
    use std::net::Ipv4Addr;

    #[test]
    fn test_encode_decode() {
        let message = Message {
            command: Command::Request,
            entries: vec![
                Entry {
                    address_family_id: 0,
                    route_tag: 10,
                    ip_address: Ipv4Addr::new(129, 21, 60, 79),
                    subnet_mask: 24,
                    metric: 10,
                },
                Entry {
                    address_family_id: 2,
                    route_tag: 20,
                    ip_address: Ipv4Addr::new(19, 23, 61, 89),
                    subnet_mask: 10,
                    metric: 2,
                },
            ],
        };

        let encoded = message.encode().unwrap();
        let decoded = Message::decode(&encoded).unwrap();

        assert_eq!(message, decoded);
    }
}
