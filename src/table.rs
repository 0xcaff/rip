use config::Config;
use config::Neighbor;
use failure::Error;
use proto::Command;
use proto::Entry;
use proto::Message;
use proto::ALLOWED_NETMASK;
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
use udp::UdpStream;
use NetworkPrefix;

struct DirectlyConnectedContext {
    address: SocketAddrV4,
    metric: u32,
}

impl<'a> From<&'a Neighbor> for DirectlyConnectedContext {
    fn from(neighbor: &'a Neighbor) -> Self {
        DirectlyConnectedContext {
            address: neighbor.ip_address,
            metric: neighbor.metric,
        }
    }
}

#[derive(Debug)]
struct Route {
    destination: Ipv4Addr,
    next_hop: Ipv4Addr,
    metric: u32,

    /// A handle to the future which resolves after a neighbor hasn't sent a message in some amount
    /// of time.
    neighbor_timeout_handle: Option<SpawnHandle<(), tokio::timer::Error>>,
}

impl Route {
    fn restart_timeout(&mut self, routing_table: Arc<Mutex<RoutingTable>>, network_prefix: u32) {
        let routing_table_clone = routing_table.clone();
        let timeout = Delay::new(Instant::now() + Duration::new(180, 0)).map(move |_a| {
            let mut routing_table = routing_table_clone.lock().unwrap();
            routing_table.handle_timeout(network_prefix)
        });

        let handle = spawn_handle(timeout);

        self.neighbor_timeout_handle = Some(handle);
    }
}

struct RoutingTable {
    table: HashMap<NetworkPrefix, Route>,
    directly_connected: HashMap<Ipv4Addr, DirectlyConnectedContext>,
    send_socket: std::net::UdpSocket,
}

impl RoutingTable {
    fn new(config: Config) -> Result<(RoutingTable, UdpStream), Error> {
        let (send_socket, recv_stream) = create_sockets(config.bind_address)?;

        let mut directly_connected = HashMap::new();

        for neighbor in &config.neighbors {
            directly_connected.insert(
                *neighbor.ip_address.ip(),
                DirectlyConnectedContext::from(neighbor),
            );
        }

        let mut table = HashMap::new();
        for neighbor in &config.neighbors {
            let network_prefix = neighbor.network_prefix();

            table.insert(
                network_prefix,
                Route {
                    destination: *neighbor.ip_address.ip(),
                    next_hop: *neighbor.ip_address.ip(),
                    metric: neighbor.metric,
                    neighbor_timeout_handle: None,
                },
            );
        }

        Ok((
            RoutingTable {
                table,
                directly_connected,
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
        for entry in entries {
            self.handle_entry(shared.clone(), from.clone(), entry)
                .map_err(|e| println!("Warning: {}", e))
                .ok();
        }

        Ok(())
    }

    fn handle_entry(
        &mut self,
        shared: Arc<Mutex<RoutingTable>>,
        from: SocketAddr,
        entry: Entry,
    ) -> Result<(), Error> {
        let addr = get_ipv4_addr(from)?;

        let context = self.directly_connected.get(&addr).ok_or_else(|| {
            format_err!(
                "received a route update from a non-neighboring node {}",
                from
            )
        })?;

        entry.validate()?;
        let network_prefix = entry.network_prefix();
        let metric_through = min(entry.metric + context.metric, 16);

        let route_to_add = {
            match self.table.get_mut(&network_prefix) {
                None if metric_through < 16 => {
                    let mut route = Route {
                        destination: entry.ip_address,
                        next_hop: *context.address.ip(),
                        metric: metric_through,
                        neighbor_timeout_handle: None,
                    };

                    route.restart_timeout(shared.clone(), network_prefix);

                    Some(route)
                }
                Some(route) => {
                    let from_next_hop = context.address.ip() == &route.next_hop;
                    if from_next_hop {
                        route.restart_timeout(shared.clone(), network_prefix);
                    }

                    if from_next_hop || metric_through < route.metric {
                        route.metric = metric_through;
                        route.next_hop = entry.next_hop;
                    }

                    None
                }
                _ => return Ok(()),
            }
        };

        if let Some(route) = route_to_add {
            self.table.insert(network_prefix, route);
        }
        self.print_table();

        Ok(())
    }

    fn print_table(&self) {
        println!("{:#?}", self.table)
    }

    fn send_updates(&self) {
        let neighbors: Vec<SocketAddrV4> = self
            .directly_connected
            .iter()
            .map(|(_, ctx)| ctx.address)
            .collect();

        neighbors.into_iter().for_each(|addr| {
            self.send_update_to(&addr)
                .map_err(|e| println!("error while sending {}", e));
        });
    }

    fn build_update_for(&self, neighbor_address: &SocketAddrV4) -> Vec<Entry> {
        let neighbor_network_prefix = u32::from(*neighbor_address.ip()) & ALLOWED_NETMASK;

        self.table
            .iter()
            .map(|(network_prefix, entry)| {
                let metric = if network_prefix == &neighbor_network_prefix {
                    16
                } else {
                    entry.metric
                };

                Entry {
                    address_family_id: 2,
                    route_tag: 0,
                    ip_address: entry.destination,
                    subnet_mask: ALLOWED_NETMASK,
                    next_hop: entry.next_hop,
                    metric,
                }
            }).collect()
    }

    fn send_update_to(&self, neighbor_address: &SocketAddrV4) -> Result<(), Error> {
        let message = Message {
            command: Command::Response,
            entries: self.build_update_for(&neighbor_address),
        };

        let encoded = message.encode()?;
        self.send_socket.send_to(&encoded, neighbor_address)?;

        Ok(())
    }

    fn handle_timeout(&mut self, network_prefix: u32) {
        println!("Something Timed Out {}", network_prefix);

        match self.table.get_mut(&network_prefix) {
            Some(entry) => {
                entry.neighbor_timeout_handle.take();
                entry.metric = 16;
            }
            _ => (),
        };

        self.print_table();
    }

    // TODO: Call Me
    fn start_timeouts(&mut self, shared: Arc<Mutex<RoutingTable>>) {
        self.table.iter_mut().for_each(|(network_prefix, entry)| {
            entry.restart_timeout(shared.clone(), *network_prefix)
        });
    }
}

pub fn start(config: Config) -> impl Future<Item = (), Error = Error> {
    RoutingTable::new(config)
        .into_future()
        .map(|(mut routing_table, recv_stream)| {
            let shared = routing_table.into_shared();

            let outbound_shared = shared.clone();
            let outbound_stream = Interval::new_interval(Duration::new(1, 0))
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
