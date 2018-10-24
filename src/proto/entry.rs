use failure::Error;
use nom::{be_u16, be_u32};
use std::net::Ipv4Addr;

use byteorder::{NetworkEndian, WriteBytesExt};

pub const ALLOWED_NETMASK: u32 = 0xffffff00;

#[derive(Eq, PartialEq, Debug)]
pub struct Entry {
    pub address_family_id: u16,
    pub route_tag: u16,
    pub ip_address: Ipv4Addr,
    pub subnet_mask: u32,
    pub metric: u32,
}

named!(pub parse_entry<Entry>, do_parse!(
    address_family_id: be_u16   >>
    route_tag:         be_u16   >>
    ip_address:        be_u32   >>
    subnet_mask:       be_u32   >>
                       take!(4) >>
    metric:            be_u32   >>

    (Entry {
        address_family_id,
        route_tag,
        ip_address: Ipv4Addr::from(ip_address),
        subnet_mask,
        metric
    })
));

impl Entry {
    pub fn decode(raw: &[u8]) -> Result<Entry, Error> {
        let (_, entry) = parse_entry(raw)
            .map_err(|err| format_err!("Failed to parse routing table entry: {}", err))?;

        Ok(entry)
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut output = Vec::new();

        // Address Family Identifier
        output.write_u16::<NetworkEndian>(self.address_family_id)?;
        output.write_u16::<NetworkEndian>(self.route_tag)?;

        output.write_u32::<NetworkEndian>(u32::from(self.ip_address))?;
        output.write_u32::<NetworkEndian>(self.subnet_mask)?;
        output.write_u32::<NetworkEndian>(0)?;
        output.write_u32::<NetworkEndian>(self.metric)?;

        Ok(output)
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.subnet_mask != ALLOWED_NETMASK {
            return Err(format_err!("invalid netmask {}", self.subnet_mask));
        };

        if !(self.metric >= 1 && self.metric <= 16) {
            return Err(format_err!("metric out of bounds {}", self.metric));
        };

        Ok(())
    }

    pub fn network_prefix(&self) -> u32 {
        u32::from(self.ip_address) & self.subnet_mask
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, Ipv4Addr};

    #[test]
    fn test_encode_decode() {
        let entry = Entry {
            address_family_id: 2,
            route_tag: 10,
            ip_address: Ipv4Addr::new(129, 21, 60, 79),
            subnet_mask: 24,
            metric: 10,
        };

        let encoded = entry.encode().unwrap();
        let decoded = Entry::decode(&encoded).unwrap();

        assert_eq!(decoded, entry);
    }
}
