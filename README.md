# rip

A partial implementation of the RIP v2 (Routing Information Protocol)
specified in RFC 2453 for a class project.

It implements everything from Section 3 of the RFC and from the Subnet Mask
section (Section 4.3) except garbage collection.

The update time is 1 second instead of 30 seconds. The timeout is 6 seconds
instead of 180 seconds. Triggered updates aren't batched.

## Running

After [installing a rust toolchain][installing-rust] with the latest stable
rust (1.28.0), run the following command from the project directory.

    cargo run -- ./config/glados.toml

After the dependencies are installed, the program should start and print the
initial routing table populated with neighbors specified in the
configuration.

## Configuration

A configuration file is used to configure information about neighbors. There
are some example config files for a simple topology in [`./config`](./config).

Here's an annotated configuration file

```toml
# The address where we expect to be contacted by neighbors and where we
# send messages to our neighbors from.
bind_address = "0.0.0.0:5892"

# List of neighboring nodes
[[neighbors]]
# Address where neighboring node can be reached.
address = "129.21.22.196:5892"

# Subnet mask of the network connected through this neighbor
subnet_mask = "255.255.255.0"

# Cost of using this route.
metric = 1

[[neighbors]]
address = "129.21.30.37:5892"
subnet_mask = "255.255.255.0"
metric = 9
```

[installing-rust]: https://www.rust-lang.org/en-US/install.html
