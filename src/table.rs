use failure::Error;

use parking_lot::ReentrantMutex;
use std;
use std::cell::RefCell;
use std::cmp::min;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, Instant};

use config::{Config, Neighbor};
use proto::{Command, Entry, Message, ALLOWED_NETMASK};
use udp::UdpStream;
use NetworkPrefix;

use tokio::prelude::*;
use tokio::{
    self,
    prelude::Future,
    reactor::Handle,
    spawn_handle,
    timer::{Delay, Interval},
    SpawnHandle,
};

pub struct RoutingTable {
    recv_stream: Option<UdpStream>,
    send_socket: std::net::UdpSocket,
    table: HashMap<NetworkPrefix, Route>,
    directly_connected: HashMap<Ipv4Addr, DirectlyConnectedContext>,
}

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

struct Route {
    destination: Ipv4Addr,
    next_hop: Ipv4Addr,
    metric: u32,

    /// A handle to the future which resolves after a neighbor hasn't sent a message in some amount
    /// of time.
    neighbor_timeout_handle: Option<SpawnHandle<(), tokio::timer::Error>>,
}

impl Route {
    fn restart_timeout(
        &mut self,
        shared_routing_table: Arc<ReentrantMutex<RefCell<RoutingTable>>>,
        network_prefix: u32,
    ) {
        let arc_mutex_clone = shared_routing_table.clone();
        let timeout = Delay::new(Instant::now() + Duration::new(180, 0)).map(move |_a| {
            let locked = arc_mutex_clone.lock();
            let mut this = locked.deref().borrow_mut();

            this.handle_timeout(network_prefix)
        });

        let handle = spawn_handle(timeout);

        self.neighbor_timeout_handle = Some(handle);
    }
}

impl RoutingTable {
    pub fn new(config: Config) -> Result<RoutingTable, Error> {
        let send_socket = std::net::UdpSocket::bind(config.bind_address)?;
        let recv_socket_sync = send_socket.try_clone()?;
        let recv_socket = tokio::net::UdpSocket::from_std(recv_socket_sync, &Handle::default())?;
        let recv_stream = UdpStream::new(recv_socket);

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

            // TODO: Timeout
        }

        Ok(RoutingTable {
            recv_stream: Some(recv_stream),
            send_socket,
            table,
            directly_connected,
        })
    }

    pub fn start(
        arc_mutex: Arc<ReentrantMutex<RefCell<Self>>>,
    ) -> impl Future<Item = (), Error = Error> {
        let arc_mutex_cloned = arc_mutex.clone();

        let outbound_future = Interval::new_interval(Duration::new(30, 0))
            .map(move |_| {
                let locked = arc_mutex_cloned.lock();
                let this = locked.deref().borrow_mut();

                let neighbors: Vec<SocketAddrV4> = this
                    .directly_connected
                    .iter()
                    .map(|(_, ctx)| ctx.address)
                    .collect();

                neighbors.into_iter().for_each(|addr| {
                    this.send_update_to(&addr)
                        .map_err(|e| println!("error while sending {}", e));

                    ()
                })
            }).filter(|_| false)
            .into_future()
            .map(|_| ())
            .map_err(|_| ());

        let locked = arc_mutex.lock();
        let mut this = locked.deref().borrow_mut();

        let other_arc_mutex_cloned = arc_mutex.clone();
        let inbound_future = this
            .recv_stream
            .take()
            .unwrap()
            .and_then(move |(from_addr, message)| {
                Self::handle_incoming(other_arc_mutex_cloned.clone(), message, from_addr)
            }).filter(|_| false)
            .map_err(|e| println!("error while handing response {}", e))
            .into_future()
            .map(|_| ())
            .map_err(|_| ());

        inbound_future
            .join(outbound_future)
            .map(|_| ())
            .map_err(|_| format_err!("future failed"))
    }

    fn handle_incoming(
        arc_mutex: Arc<ReentrantMutex<RefCell<Self>>>,
        message: Message,
        from: SocketAddr,
    ) -> Result<(), Error> {
        match message.command {
            Command::Request => {
                // Ignore requests, used mainly for diagnostics. Not required by assignment.
            }
            Command::Response => {
                Self::handle_response(arc_mutex, message.entries, from)?;
            }
        };

        Ok(())
    }

    fn handle_response(
        arc_mutex: Arc<ReentrantMutex<RefCell<Self>>>,
        entries: Vec<Entry>,
        from: SocketAddr,
    ) -> Result<(), Error> {
        let locked = arc_mutex.lock();
        let this = locked.deref().borrow_mut();

        let addr = get_ipv4_addr(from)?;
        let context = this
            .directly_connected
            .get(&addr)
            .ok_or_else(|| format_err!("received a route update from a non-neighboring node"))?;

        for entry in entries {
            Self::handle_entry(arc_mutex.clone(), entry, context)
                .map_err(|e| println!("Warning: {}", e))
                .ok();
        }

        Ok(())
    }

    fn handle_entry(
        arc_mutex: Arc<ReentrantMutex<RefCell<Self>>>,
        entry: Entry,
        context: &DirectlyConnectedContext,
    ) -> Result<(), Error> {
        let locked = arc_mutex.lock();
        let mut this = locked.deref().borrow_mut();

        entry.validate()?;
        let network_prefix = entry.network_prefix();
        let metric_through = min(entry.metric + context.metric, 16);

        let route_to_add = {
            match this.table.get_mut(&network_prefix) {
                None if metric_through < 16 => {
                    let mut route = Route {
                        destination: entry.ip_address,
                        next_hop: *context.address.ip(),
                        metric: metric_through,
                        neighbor_timeout_handle: None,
                    };

                    route.restart_timeout(arc_mutex.clone(), network_prefix);

                    Some(route)
                }
                Some(route) => {
                    let from_next_hop = context.address.ip() == &route.next_hop;
                    if from_next_hop {
                        route.restart_timeout(arc_mutex.clone(), network_prefix);
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
            this.table.insert(network_prefix, route);
        }

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
}

fn get_ipv4_addr(from: SocketAddr) -> Result<Ipv4Addr, Error> {
    match from.ip() {
        IpAddr::V4(addr) => Ok(addr),
        _ => return Err(format_err!("not an ipv4 address {}", from)),
    }
}
