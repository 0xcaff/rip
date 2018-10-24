use config::Config;
use config::Neighbor;
use failure::Error;
use proto::Command;
use proto::Entry;
use proto::Message;
use proto::UdpStream;
use std;
use std::cmp::min;
use std::collections::HashMap;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use tokio;
use tokio::prelude::*;
use tokio::reactor::Handle;
use tokio::spawn_handle;
use tokio::timer::Delay;
use tokio::timer::Interval;
use tokio::SpawnHandle;
use NetworkPrefix;

const REFRESH_DELAY: u64 = 1;
const TIMEOUT_DELAY: u64 = REFRESH_DELAY * 6;

#[derive(Debug)]
struct Route {
    destination: Ipv4Addr,
    next_hop: Ipv4Addr,
    subnet_mask: Ipv4Addr,
    metric: u32,

    /// A handle to the future which resolves after a neighbor hasn't sent a message in some amount
    /// of time.
    neighbor_timeout_handle: Option<SpawnHandle<(), tokio::timer::Error>>,
}

impl Route {
    fn as_entry(&self, neighbor: &Ipv4Addr) -> Entry {
        let metric = if &self.destination == neighbor || &self.next_hop == neighbor {
            16
        } else {
            self.metric
        };

        Entry {
            address_family_id: 2,
            route_tag: 0,
            ip_address: self.destination,
            subnet_mask: u32::from(self.subnet_mask),
            metric,
        }
    }

    fn restart_timeout(&mut self, routing_table: Arc<Mutex<RoutingTable>>, network_prefix: u32) {
        let timeout = Delay::new(Instant::now() + Duration::new(TIMEOUT_DELAY, 0)).map(move |_a| {
            let mut routing_table = routing_table.lock().unwrap();
            routing_table.handle_timeout(network_prefix)
        });

        let handle = spawn_handle(timeout);

        self.neighbor_timeout_handle.take().map(|v| v.cancel());
        self.neighbor_timeout_handle = Some(handle);
    }
}

struct RoutingTable {
    table: HashMap<NetworkPrefix, Route>,
    neighbors: HashMap<Ipv4Addr, Neighbor>,
    send_socket: std::net::UdpSocket,
}

impl RoutingTable {
    fn new(config: Config) -> Result<(RoutingTable, UdpStream), Error> {
        let (send_socket, recv_stream) = create_sockets(config.bind_address)?;

        let mut table = HashMap::new();
        for neighbor in &config.neighbors {
            let network_prefix = neighbor.network_prefix();

            table.insert(
                network_prefix,
                Route {
                    destination: *neighbor.address.ip(),
                    next_hop: *neighbor.address.ip(),
                    subnet_mask: neighbor.subnet_mask,
                    metric: neighbor.metric,
                    neighbor_timeout_handle: None,
                },
            );
        }

        let mut directly_connected = HashMap::new();
        for neighbor in config.neighbors {
            directly_connected.insert(*neighbor.address.ip(), neighbor);
        }

        Ok((
            RoutingTable {
                table,
                neighbors: directly_connected,
                send_socket,
            },
            recv_stream,
        ))
    }

    fn into_shared(self) -> Arc<Mutex<RoutingTable>> {
        Arc::new(Mutex::new(self))
    }

    fn handle_incoming(
        &mut self,
        shared: Arc<Mutex<RoutingTable>>,
        message: Message,
        from: SocketAddr,
    ) -> Result<(), Error> {
        match message.command {
            Command::Request => {
                // Ignore requests, used mainly for diagnostics. Not required by assignment.
            }
            Command::Response => {
                self.handle_response(shared, message.entries, from)?;
            }
        };

        Ok(())
    }

    fn handle_response(
        &mut self,
        shared: Arc<Mutex<RoutingTable>>,
        entries: Vec<Entry>,
        from: SocketAddr,
    ) -> Result<(), Error> {
        let from_addr = get_ipv4_addr(from)?;

        let neighbor = self
            .neighbors
            .get(&from_addr)
            .ok_or_else(|| {
                format_err!(
                    "received a route update from a non-neighboring node {}",
                    from_addr
                )
            })?.clone();

        self.handle_entry(
            shared.clone(),
            neighbor.clone(),
            Entry {
                address_family_id: 2,
                route_tag: 0,
                ip_address: from_addr.clone(),
                subnet_mask: u32::from(neighbor.subnet_mask),
                metric: 0,
            },
        );

        for entry in entries {
            entry.validate()?;

            self.handle_entry(shared.clone(), neighbor.clone(), entry);
        }

        Ok(())
    }

    fn handle_entry(&mut self, shared: Arc<Mutex<RoutingTable>>, neighbor: Neighbor, entry: Entry) {
        let network_prefix = entry.network_prefix();
        let src_to_dst_metric = min(neighbor.metric + entry.metric, 16);

        let (route_to_add, should_trigger_update) = {
            match self.table.get_mut(&network_prefix) {
                None if src_to_dst_metric < 16 => {
                    let mut route = Route {
                        destination: entry.ip_address,
                        next_hop: *neighbor.address.ip(),
                        subnet_mask: Ipv4Addr::from(entry.subnet_mask),
                        metric: src_to_dst_metric,
                        neighbor_timeout_handle: None,
                    };

                    route.restart_timeout(shared.clone(), network_prefix);

                    (Some(route), false)
                }
                Some(ref mut route) => {
                    let is_neighbor = &route.next_hop == neighbor.address.ip();
                    if is_neighbor || src_to_dst_metric < route.metric {
                        route.metric = src_to_dst_metric;
                        route.next_hop = *neighbor.address.ip();

                        route.restart_timeout(shared.clone(), network_prefix);

                        (None, src_to_dst_metric < route.metric)
                    } else {
                        (None, false)
                    }
                }
                _ => return,
            }
        };

        if let Some(route) = route_to_add {
            self.send_triggered_update(&route);
            self.table.insert(network_prefix, route);
        }

        if should_trigger_update {
            self.send_triggered_update_for(&network_prefix);
        }

        self.print_table();
    }

    fn print_table(&self) {
        println!(
            "| {:20} | {:20} | {:20} | {:6} |",
            "Destination", "Subnet Mask", "Next Hop", "Metric"
        );

        for (_, entry) in &self.table {
            println!(
                "| {:20} | {:20} | {:20} | {:<6} |",
                entry.destination.to_string(),
                entry.subnet_mask.to_string(),
                entry.next_hop.to_string(),
                entry.metric
            );
        }

        println!();
    }

    fn send_updates(&self) {
        let neighbors: Vec<SocketAddrV4> =
            self.neighbors.iter().map(|(_, ctx)| ctx.address).collect();

        neighbors.into_iter().for_each(|addr| {
            self.send_entries_to(&addr, self.build_update_for(&addr))
                .map_err(|e| println!("error while sending {}", e))
                .ok();
        });
    }

    fn send_triggered_update_for(&self, network_prefix: &u32) {
        let route = self.table.get(&network_prefix).unwrap();

        self.send_triggered_update(route);
    }

    fn send_triggered_update(&self, route: &Route) {
        println!("Triggering Update for {}", route.destination);

        for (_, neighbor) in &self.neighbors {
            let entry = route.as_entry(neighbor.address.ip());
            self.send_entries_to(&neighbor.address, vec![entry])
                .map_err(|e| println!("Error while sending triggered update {}", e))
                .ok();
        }
    }

    fn build_update_for(&self, neighbor_address: &SocketAddrV4) -> Vec<Entry> {
        let neighbor_ip = neighbor_address.ip();

        self.table
            .iter()
            .map(|(_network_prefix, entry)| entry.as_entry(neighbor_ip))
            .collect()
    }

    fn send_entries_to(
        &self,
        neighbor_address: &SocketAddrV4,
        entries: Vec<Entry>,
    ) -> Result<(), Error> {
        let message = Message {
            command: Command::Response,
            entries,
        };

        let encoded = message.encode()?;
        self.send_socket.send_to(&encoded, neighbor_address)?;

        Ok(())
    }

    fn handle_timeout(&mut self, network_prefix: u32) {
        let should_trigger_update = match self.table.get_mut(&network_prefix) {
            Some(entry) => {
                entry.neighbor_timeout_handle.take();
                entry.metric = 16;

                true
            }
            _ => false,
        };

        if should_trigger_update {
            self.send_triggered_update_for(&network_prefix);
        }

        self.print_table();
    }

    fn start_timeouts(&mut self, shared: Arc<Mutex<RoutingTable>>) {
        self.table.iter_mut().for_each(|(network_prefix, entry)| {
            entry.restart_timeout(shared.clone(), *network_prefix)
        });
    }
}

pub fn start(config: Config) -> impl Future<Item = (), Error = Error> {
    RoutingTable::new(config)
        .into_future()
        .map(|(routing_table, recv_stream)| {
            routing_table.print_table();
            let shared = routing_table.into_shared();

            {
                let mut routing_table = shared.lock().unwrap();
                routing_table.start_timeouts(shared.clone());
            }

            let outbound_shared = shared.clone();
            let outbound_stream = Interval::new_interval(Duration::new(REFRESH_DELAY, 0))
                .map(move |_| {
                    let routing_table = outbound_shared.lock().unwrap();
                    routing_table.send_updates();
                }).map_err(|e| Error::from(e));

            let inbound_stream = recv_stream.and_then(move |(from_addr, message)| {
                let mut routing_table = shared.lock().unwrap();
                routing_table.handle_incoming(shared.clone(), message, from_addr)
            });

            outbound_stream
                .select(inbound_stream)
                .filter(|_| false)
                .into_future()
                .map_err(|e| e.0)
                .map(|_| ())
        }).flatten()
}

fn create_sockets<A: ToSocketAddrs>(address: A) -> Result<(std::net::UdpSocket, UdpStream), Error> {
    let send_socket = std::net::UdpSocket::bind(address)?;
    let recv_socket_sync = send_socket.try_clone()?;
    let recv_socket = tokio::net::UdpSocket::from_std(recv_socket_sync, &Handle::default())?;
    let recv_stream = UdpStream::new(recv_socket);

    Ok((send_socket, recv_stream))
}

fn get_ipv4_addr(from: SocketAddr) -> Result<Ipv4Addr, Error> {
    match from.ip() {
        IpAddr::V4(addr) => Ok(addr),
        _ => return Err(format_err!("not an ipv4 address {}", from)),
    }
}
