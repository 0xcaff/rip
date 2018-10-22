use failure::Error;

use parking_lot::ReentrantMutex;
use std::cell::RefCell;
use std::cmp::min;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{Command, Entry, Message};
use NetworkPrefix;

use tokio::{self, prelude::Future, spawn_handle, timer::Delay, SpawnHandle};

struct RoutingTable {
    send_socket: UdpSocket,
    table: HashMap<NetworkPrefix, Route>,
    directly_connected: HashMap<Ipv4Addr, DirectlyConnectedContext>,
}

struct DirectlyConnectedContext {
    address: Ipv4Addr,
    metric: u32,
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
            let mut this = locked.borrow_mut();

            this.handle_timeout(network_prefix)
        });

        let handle = spawn_handle(timeout);

        self.neighbor_timeout_handle = Some(handle);
    }
}

impl RoutingTable {
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
        let this = locked.borrow_mut();

        let addr = get_ipv4_addr(from)?;
        let context = this
            .directly_connected
            .gt(&addr)
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
        let mut this = locked.borrow_mut();

        entry.validate()?;
        let network_prefix = entry.network_prefix();
        let metric_through = min(entry.metric + context.metric, 16);

        let route_to_add = {
            match this.table.get_mut(&network_prefix) {
                None if metric_through < 16 => {
                    let mut route = Route {
                        destination: entry.ip_address,
                        next_hop: context.address,
                        metric: metric_through,
                        neighbor_timeout_handle: None,
                    };

                    route.restart_timeout(arc_mutex.clone(), network_prefix);

                    Some(route)
                }
                Some(route) => {
                    let from_next_hop = context.address == route.next_hop;
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
        let neighbor_network_prefix = u32::from(neighbor_address.ip()) & ALLOWED_NETMASK;

        self.table
            .iter()
            .map(|(network_prefix, entry)| {
                let metric = if network_prefix == neighbor_network_prefix {
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

    fn send_update_to(&self, neighbor_address: SocketAddrV4) -> Result<(), Error> {
        let message = Message {
            command: Command::Response,
            entries: self.build_update_for(&neighbor_address),
        };

        let encoded = message.encode()?;
        self.send_socket
            .send_to(&encoded, SocketAddr::V4(neighbor_address))?;

        Ok(())
    }
}

fn get_ipv4_addr(from: SocketAddr) -> Result<Ipv4Addr, Error> {
    match from.ip() {
        IpAddr::V4(addr) => Ok(addr),
        _ => return Err(format_err!("not an ipv4 address {}", from)),
    }
}
