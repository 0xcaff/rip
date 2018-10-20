use failure::Error;
use nom::{be_u16, be_u32};
use std::net::Ipv4Addr;

use byteorder::{NetworkEndian, WriteBytesExt};

#[derive(Eq, PartialEq, Debug)]
pub struct Entry {
    pub address_family_id: u16,
    pub route_tag: u16,
    pub ip_address: Ipv4Addr,
    pub subnet_mask: u32,
    pub next_hop: Ipv4Addr,
    pub metric: u32,
}

named!(pub parse_entry<Entry>, do_parse!(
    address_family_id: be_u16 >>
    route_tag:         be_u16 >>
    ip_address:        be_u32 >>
    subnet_mask:       be_u32 >>
    next_hop:          be_u32 >>
    metric:            be_u32 >>

    (Entry {
        address_family_id,
        route_tag,
        ip_address: Ipv4Addr::from(ip_address),
        subnet_mask,
        next_hop: Ipv4Addr::from(next_hop),
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
        output.write_u32::<NetworkEndian>(u32::from(self.next_hop))?;
        output.write_u32::<NetworkEndian>(self.metric)?;

        Ok(output)
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
            next_hop: Ipv4Addr::new(123, 94, 63, 21),
            metric: 10,
        };

        let encoded = entry.encode().unwrap();
        let decoded = Entry::decode(&encoded).unwrap();

        assert_eq!(decoded, entry);
    }
}
