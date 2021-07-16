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
* Add support for retrieving historical auction information via the addition of an optional `--block-identifier` arg in the `get-auction-info` subcommand.
* Add `keygen::generate_files` to FFI.

### Changed
* Change `account-address` subcommand to output properly formatted string.
* Change `put-deploy` and `make-deploy` subcommands to support transfers.
* Change `make-deploy`, `make-transfer` and `sign-deploy` to not overwrite files unless `--force` is passed.
* Change `make-deploy`, `make-transfer` and `sign-deploy` to use transactional file writing for enhanced safety and reliability.
* Update pinned version of Rust to `nightly-2021-06-17`


## [1.2.0] - 2021-05-27

### Added
* Support multisig transfers via new `make-transfer` subcommand.

### Changed
* Change to Apache 2.0 license.
* Make `--transfer-id` a required argument of the relevant subcommands.
* Reduce deploy default time-to-live to 30 minutes.



## [1.1.1] - 2021-04-19

No changes.



## [1.1.0] - 2021-04-13 [YANKED]

No changes.



## [1.0.1] - 2021-04-08

### Changed
* Fail if creating a deploy greater than 1 MiB.



## [1.0.0] - 2021-03-30

### Added
* Initial release of client compatible with Casper mainnet.



[Keep a Changelog]: https://keepachangelog.com/en/1.0.0
[unreleased]: https://github.com/casper-network/casper-node/compare/v1.2.0...dev
[1.2.0]: https://github.com/casper-network/casper-node/compare/v1.1.1...v1.2.0
[1.1.1]: https://github.com/casper-network/casper-node/compare/v1.0.1...v1.1.1
[1.1.0]: https://github.com/casper-network/casper-node/compare/v1.0.1...v1.1.1
[1.0.1]: https://github.com/casper-network/casper-node/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/casper-network/casper-node/releases/tag/v1.0.0
