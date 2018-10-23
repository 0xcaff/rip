use failure::Error;

use std::net::SocketAddr;

use tokio::net::UdpSocket;
use tokio::prelude::*;

use proto::Message;

pub struct UdpStream {
    socket: UdpSocket,
}

impl UdpStream {
    pub fn new(socket: UdpSocket) -> UdpStream {
        UdpStream { socket }
    }
}

impl Stream for UdpStream {
    type Item = (SocketAddr, Message);
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let mut buffer = [0u8; 25 * 20 + 4];
        let (size, from_addr) = try_ready!(self.socket.poll_recv_from(&mut buffer));

        let message = Message::decode(&buffer[..size])?;

        Ok(Async::Ready(Some((from_addr, message))))
    }
}
