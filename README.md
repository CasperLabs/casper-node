[![LOGO](images/casper-logo-primary.png)](https://casper.network)

Casper is the blockchain platform purpose-built to scale opportunity for everyone. Building toward blockchain’s next frontier, Casper is designed for real-world applications without sacrificing usability, cost, decentralization, or security. It removes the barriers that prevent mainstream blockchain adoption by making blockchain friendly to use, open to the world, and future-proof to support innovations today and tomorrow. Guided by open-source principles and built from the ground up to empower individuals, the team seeks to provide an equitable foundation made for long-lasting impact. Read more at: https://casper.network

## Current Development Status
The status on development is reported during the Community calls and is found [here](https://github.com/casper-network/Governance/wiki/Current-Status)

The Casper Mainnet is live.
- [Clarity Block Exporer](https://clarity.casperlabs.io)

## Specification

- [Platform Specification](https://docs.casperlabs.io/en/latest/implementation/index.html)
- [Highway Consensus Proofs](https://github.com/casper-network/highway/releases/latest)

## Get Started with Smart Contracts
- [Writing Smart Contracts](https://docs.casperlabs.io/en/latest/dapp-dev-guide/index.html)
- [Rust Smart Contract SDK](https://crates.io/crates/cargo-casper)
- [Rust Smart Contract API Docs](https://docs.rs/casper-contract/latest/casper_contract/contract_api/index.html)
- [AssemblyScript Smart Contract API](https://www.npmjs.com/package/@casperlabs/contract)

## Community

- [Discord Server](https://discord.gg/mpZ9AYD)
- [CasperLabs Community Forum](https://forums.casperlabs.io/)
- [Telegram Channel](https://t.me/casperblockchain)

# casper-node

This is the core application for the Casper blockchain.

## Pre-Requisites for Building

* CMake 3.1.4 or greater
* [Rust](https://www.rust-lang.org/tools/install)
* libssl-dev
* pkg-config
* gcc
* g++
* optionally [wasm-strip](https://github.com/WebAssembly/wabt) (used to reduce the size of compiled Wasm)

## Connecting to Mainnet

The community has come up with a guide for connecting to mainnet from scratch:

https://github.com/make-software/how-to-casperlabs/blob/master/docs/ubuntu/setup-validator-from-scratch.md

### Setup

Before building a node, prepare your Rust build environment:

```
make setup-rs
```

The node software can be compiled afterwards:

```
cargo build -p casper-node --release
```

The result will be a `casper-node` binary found in `target/release`.  Copy this somewhere into your
PATH, or substitute `target/release/casper-node` for `casper-node` in all examples below.


To run a validator node you will need to specify a config file and launch the validator subcommand, for example

```
casper-node validator /etc/casper-node/config.toml
```

The node ships with an [example configuration file](resources/local/config.toml) that should be setup first.  There is
also a template for a local [chainspec](resources/local/chainspec.toml.in) in the same folder.

For launching, the following configuration values must be properly set:

| Setting                   | Description |
| :-------------------------| :---------- |
| `network.known_addresses` | Must refer to public listening addresses of one or more currently-running nodes.  If the node cannot connect to any of these addresses, it will panic.  The node _can_ be run with this referring to its own address, but it will be equivalent to specifying an empty list for `known_addresses` - i.e. the node will run and listen, but will be reliant on other nodes connecting to it in order to join the network.  This would be normal for the very first node of a network, but all subsequent nodes should normally specify that first  node's public listening address as their `known_addresses`. |

__The node will not run properly without another node to connect to.  It is recommended that multiple nodes are run.__

### Running multiple nodes on one machine

There is a [tool](https://github.com/casper-network/casper-node/tree/master/utils/nctl) which automates the process of running multiple nodes on a single machine.

Note that running multiple nodes on a single machine is normally only recommended for test purposes.

## Configuration

In general nodes are configured through a configuration file, typically named `config.toml`.  This
file may reference other files or locations through relative paths.  When it does, note that all
paths that are not absolute will be resolved relative to `config.toml` directory.


### Environment overrides

Some environments may call for overriding options through the environment.  In this
scenario, the `NODE_CONFIG` environment variable can be used. For example:
alternatively expressed as

```
export NODE_CONFIG=consensus.secret_key_path=secret_keys/node-1.pem;network.known_addresses=[1.2.3.4:34553, 200.201.203.204:34553]
casper-node validator /etc/casper-node/config.toml
```

Note how the semicolon is used to separate configuration overrides here.

### Development environment variables

To set the threshold at which a warn-level log message is generated for a long-running reactor event, use the env var
`CL_EVENT_MAX_MICROSECS`.  For example, to set the threshold to 1 millisecond:

```
CL_EVENT_MAX_MICROSECS=1000
```

To set the threshold at which a queue dump will occur, use the env var `CL_MEM_DUMP_THRESHOLD_MB`. When the process reaches this level of memory allocation a dump will occur, but this will only occur once. Queue dumps can be found in `/tmp` once they are complete. For example, to set the threshold to 16000 megabytes:

```
CL_MEM_DUMP_THRESHOLD_MB=16000
```


## Logging

Logging can be enabled by setting the environment variable `RUST_LOG`.  This can be set to one of the following levels,
from lowest priority to highest: `trace`, `debug`, `info`, `warn`, `error`:

```
RUST_LOG=info cargo run --release -- validator resources/local/config.toml
```

If the environment variable is unset, it is equivalent to setting `RUST_LOG=error`.

#### Log message format

A typical log message will look like:

```
Jun 09 01:40:17.315 INFO  [casper_node::components::rpc_server rpc_server.rs:127] starting HTTP server; server_addr=127.0.0.1:7777
```

This is comprised of the following parts:
* timestamp
* log level
* full module path (not to be confused with filesystem path) of the source of the message
* filename and line number of the source of the message
* message

#### Filtering log messages

`RUST_LOG` can be set to enable varying levels for different modules.  Simply set it to a comma-separated list of
`module-path=level`, where the module path is as shown above in the typical log message, with the end truncated to suit.

For example, to enable `trace` level logging for the `small_network` module in `components`, `info` level for all other
modules in `components`, and `warn` level for the remaining codebase:

```
RUST_LOG=casper_node::components::small=trace,casper_node::comp=info,warn
```

## Debugging

Some additional debug functionality is available, mainly allowed for inspections of the internal event queue.

### Event queue dump

The event queue can be dumped by sending a `SIGUSR1` to the running node process, e.g. if the node's process ID was `$NODE_PID`:

```console
kill -USR1 $NODE_PID
```

This will create a `queue_dump.json` in the working directory of the node. A tool like [jq](https://stedolan.github.io/jq/) can then be used to format and display it:

```console
$ jq < queue_dump.json
{
  "NetworkIncoming": [],
  "Network": [],
  "Regular": [
    "AddressGossiper"
  ],
  "Api": []
}
```

#### jq Examples

Dump the type of events:

```console
jq 'map_values( map(keys[0] | {"type": ., weight: 1})| group_by(.type) | map ([.[0].type,(.|length)]) | map({(.[0]): .[1]}) )' queue_dump.json
```

Count number of events in each queue:

```console
jq 'map_values(map(keys[0]))' queue_dump.json
```

## Running a client

See [the client README](client/README.md).

## Running a local network

See [the nctl utility README](utils/nctl/README.md).
