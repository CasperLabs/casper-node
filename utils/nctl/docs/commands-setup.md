# NCTL Setup Commands

## Overview

The aim of NCTL is to enable a user to spin up a test network within 15-20 seconds.  Once a network is up & running the user should be able to control each of the node's within the network as well as add new nodes to the network.  Hereby are listed the set of NCTL commands to setup assets (binaries, config files, directories ... etc ) associated with a test network.

## Compiling network binaries

The NCTL library can be used to compile the node's binary set, i.e. node, client & smart contract binaries.  Note that NCTL library does not immediately copy compiled binary sets into a test directory, that is done whilst setting up test assets (see `nctl-assets-setup` below). 

### nctl-compile

Compiles casper node, client + client contracts using `make` + `cargo`.  


### nctl-compile-node

Compiles casper node using `make` + `cargo`.  


### nctl-compile-client

Compiles casper client + client contracts using `make` + `cargo`.  


## Managing network assets

### nctl-assets-ls

List previously created network assets.


### nctl-assets-setup net={W:-1} nodes={X:-5} users={Y:-5} bootstraps={Z:-1}

Sets up assets required to run a local network - this includes binaries, chainspec, config, faucet, keys ... etc.  NCTL creates assets for 2 nodesets: genesis & non-genesis - this permits testing nodeset rotation scenarios (see `nctl-rotate`). 

```
nctl-assets-setup

nctl-assets-setup net=1 nodes=5 users=5 bootstraps=1  (same as above)

nctl-assets-setup net=2 nodes=10 users=10 bootstraps=4
```

### nctl-assets-teardown net={X:-1}

Stops network & destroys all related assets.

```
nctl-assets-teardown

nctl-assets-teardown net=1  (same as above)

nctl-assets-teardown net=2
```

### nctl-assets-dump net={X:-1}

Dumps transient network assets such as logs + configuration.

```
nctl-assets-dump

nctl-assets-dump net=1  (same as above)
```
