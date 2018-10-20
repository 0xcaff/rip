use std::net::Ipv4Addr;
use nom::{be_u16, be_u32, IResult};
use failure::Error;

use byteorder::NetworkEndian;
use byteorder::WriteBytesExt;

#[derive(Eq, PartialEq, Debug)]
pub struct Entry {
    pub route_tag: u16,
    pub ip_address: Ipv4Addr,
    pub subnet_mask: u32,
    pub next_hop: Ipv4Addr,
    pub metric: u32,
}

impl Entry {
    fn parse_nom(input: &[u8]) -> IResult<&[u8], Entry> {
        do_parse!(input,
                         take!(2) >>
            route_tag:   be_u16   >>
            ip_address:  be_u32   >>
            subnet_mask: be_u32   >>
            next_hop:    be_u32   >>
            metric:      be_u32   >>
            (Entry {
                route_tag,
                ip_address: Ipv4Addr::from(ip_address),
                subnet_mask,
                next_hop: Ipv4Addr::from(next_hop),
                metric
            })
        )
    }

    pub fn decode(raw: &[u8]) -> Result<Entry, Error> {
        let (_, entry) = Entry::parse_nom(raw)
            .map_err(|_e| format_err!("Failed to parse routing table entry."))?;

        Ok(entry)
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut output = Vec::new();

        // Address Family Identifier
        output.write_u16::<NetworkEndian>(2)?;
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

