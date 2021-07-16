# Changelog

All notable changes to this project will be documented in this file.  The format is based on [Keep a Changelog].

[comment]: <> (Added:      new features)
[comment]: <> (Changed:    changes in existing functionality)
[comment]: <> (Deprecated: soon-to-be removed features)
[comment]: <> (Removed:    now removed features)
[comment]: <> (Fixed:      any bug fixes)
[comment]: <> (Security:   in case of vulnerabilities)



## [Unreleased]

### Added
* Add support for providing historical auction information via the addition of an optional block ID in the `state_get_auction_info` JSON-RPC. 
* Exclude inactive validators from proposing blocks.
* Add validation of the `[protocol]` configuration on startup, to ensure the contained values make sense.
* Add optional outgoing bandwidth limiter to the networking component, controllable via new `[network][max_outgoing_byte_rate_non_validators]` config option.
* Add optional incoming message limiter to the networking component, controllable via new `[network][max_incoming_message_rate_non_validators]` config option.
* Add optional in-memory deduplication of deploys, controllable via new `[storage]` config options `[enable_mem_deduplication]` and `[mem_pool_prune_interval]`.
* Add a new event stream to SSE server accessed via `<IP:Port>/events/deploys` which emits deploys in full as they are accepted.

### Changed
* Major rewrite of the network component, covering connection negotiation and management, periodic housekeeping and logging.
* Exchange and authenticate Validator public keys in network handshake between peers.
* Remove needless copying of outgoing network messages.
* Move finality signatures to separate event stream and change stream endpoints to `/events/main` and `/events/sigs`.
* Avoid truncating the state root hash when reporting node's status via JSON-RPC or REST servers.
* The JSON-RPC server waits until an incoming deploy has been sent to storage before responding to the client.
* Persist event stream event index across node restarts.
* Separate transfers from other deploys in the block proposer.
* Enable getting validators for future eras in `EffectBuilder::get_era_validators()`.
* Replace config option `[block_propser][deploy_delay]` (which specified a fixed delay before proposing a deploy) with a gossip-finished announcement.
* Improve logging around stalled consensus detection.
* Skip storage integrity checks if the node didn't previously crash.
* Update pinned version of Rust to `nightly-2021-06-17`
* Changed LMDB flags to reduce flushing and optimize IO performance in the Contract Runtime.
* Don't shut down by default anymore if stalled. To enable set config option `shutdown_on_standstill = true` in `[consensus.highway]`.

### Removed
* Remove systemd notify support, including removal of `[network][systemd_support]` config option.

### Fixed
* Avoid redundant requests caused by the Highway synchronizer.
* Update "current era" metric also for initial era.
* Keep syncing until the node is in the current era, rather than allowing an acceptable drift.
* Update the list of peers with newly-learned ones in linear chain sync.
* Drain the joiner reactor queue on exit, to eliminate stale connections whose handshake has completed, but which live on the queue.
* Shut down SSE event streams gracefully.
* Limit the maximum number of clients connected to the event stream server via the `[event_stream_server][max_concurrent_subscribers]` config option.
* Avoid emitting duplicate events in the event stream.



## [1.2.0] - 2021-05-27

### Added
* Add configuration options for `[consensus][highway][round_success_meter]`.
* Add `[protocol][last_emergency_restart]` field to the chainspec for use by fast sync.
* Add an endpoint at `/rpc-schema` to the REST server which returns the OpenRPC-compatible schema of the JSON-RPC API.
* Have consensus component shut down the node on an absence of messages for the last era for a given period.
* Add a new `Step` event to the event stream which displays the contract runtime `Step` execution results.
* Add a configurable delay before proposing dependencies, to give deploys time to be gossiped before inclusion in a new block.
* Add instrumentation to the network component.
* Add fetchers for block headers.
* Add joiner test.

### Changed
* Change to Apache 2.0 license.
* Provide an efficient way of finding the block to which a given deploy belongs.
* On hard-reset upgrades, only remove stored blocks with old protocol versions, and remove all data associated with a removed block.
* Restrict expensive startup integrity checks to sessions following unclean shutdowns.
* Improve node joining process.
* Improve linear chain component, including cleanups and optimized handling of finality signatures.
* Make the syncing process, linear chain component and joiner reactor not depend on the Era Supervisor.
* Improve logging of banned peers.
* Change trigger for upgrade checks to timed interval rather than announcement of new block.
* Use the same JSON representation for a block in the event stream as for the JSON-RPC server.
* Avoid creating a new era when shutting down for an upgrade.
* Allow consensus to disconnect from faulty peers.
* Use own most recent round exponent instead of the median when initializing a new era.
* Request protocol state from peers only for the latest era.
* Add an instance ID to consensus pings, so that they are only handled in the era and the network they were meant for.
* Avoid requesting a consensus dependency that is already in the synchronizer queue.
* Change replay detection to not use execution results.
* Initialize consensus round success meter with current timestamp.
* Era Supervisor now accounts for the last emergency restart.
* Upgrade dependencies, in particular tokio.

### Removed
* Remove `impl Sub<Timestamp> for Timestamp` to help avoid panicking in non-obvious edge cases.
* Remove `impl Sub<TimeDiff> for Timestamp` from production code to help avoid panicking in non-obvious edge cases.
* Remove `[event_stream_server][broadcast_channel_size]` from config.toml, and make it a factor of the event stream buffer size.

### Fixed
* Have casper-node process exit with the exit code returned by the validator reactor.
* Restore cached block proposer state correctly.
* Runtime memory estimator now registered in the joiner reactor.
* Avoid potential arithmetic overflow in consensus component.
* Avoid potential index out of bounds error in consensus component.
* Avoid panic on dropping an event responder.
* Validate each block size in the block validator component.
* Prevent deploy replays.
* Ensure finality signatures received after storing a block are gossiped and stored.
* Ensure isolated bootstrap nodes attempt to reconnect properly.
* Ensure the reactor doesn't skip fatal errors before successfully exiting.
* Collect only verified signatures from bonded validators.
* Fix a race condition where new metrics were replaced before the networking component had shut down completely, resulting in a panic.
* Ensure an era is not activated twice.
* Avoid redundant requests caused by the Highway synchronizer.
* Reduce duplication in block validation requests made by the Highway synchronizer.
* Request latest consensus state only if consensus has stalled locally.



## [1.1.1] - 2021-04-19

### Changed
* Ensure consistent validation when adding deploys and transfers while proposing and validating blocks. 



## [1.1.0] - 2021-04-13 [YANKED]

### Changed
* Ensure that global state queries will only be permitted to recurse to a fixed maximum depth.



## [1.0.1] - 2021-04-08

### Added
* Add `[deploys][max_deploy_size]` to chainspec to limit the size of valid deploys.
* Add `[network][maximum_net_message_size]` to chainspec to limit the size of peer-to-peer messages.

### Changed
* Check deploy size does not exceed maximum permitted as part of deploy validation.
* Include protocol version and maximum message size in network handshake of nodes.
* Change accounts.toml to only be included in v1.0.0 configurations.



## [1.0.0] - 2021-03-30

### Added
* Initial release of node for Casper mainnet.



[Keep a Changelog]: https://keepachangelog.com/en/1.0.0
[unreleased]: https://github.com/casper-network/casper-node/compare/v1.2.0...dev
[1.2.0]: https://github.com/casper-network/casper-node/compare/v1.1.1...v1.2.0
[1.1.1]: https://github.com/casper-network/casper-node/compare/v1.0.1...v1.1.1
[1.1.0]: https://github.com/casper-network/casper-node/compare/v1.0.1...v1.1.1
[1.0.1]: https://github.com/casper-network/casper-node/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/casper-network/casper-node/releases/tag/v1.0.0
