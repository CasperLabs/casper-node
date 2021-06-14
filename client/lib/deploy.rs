use std::{
    fs::File,
    io::{self, BufReader, Read, Write},
};

use serde::{Deserialize, Serialize};

use casper_execution_engine::core::engine_state::ExecutableDeployItem;
use casper_node::{
    rpcs::{account::PutDeploy, chain::GetBlockResult, info::GetDeploy, RpcWithParams},
    types::{Deploy, DeployHash, TimeDiff, Timestamp},
};
use casper_types::{ProtocolVersion, RuntimeArgs, SecretKey, URef, U512};

use crate::{
    error::{Error, Result},
    rpc::{RpcClient, TransferTarget},
};

/// The maximum permissible size in bytes of a Deploy when serialized via `ToBytes`.
///
/// Note: this should be kept in sync with the value of `[deploys.max_deploy_size]` in the
/// production chainspec.
const MAX_SERIALIZED_SIZE: u32 = 1_024 * 1_024;

/// SendDeploy allows sending a deploy to the node.
pub(crate) struct SendDeploy;

/// Transfer allows transferring an amount between accounts.
pub(crate) struct Transfer {}

impl RpcClient for PutDeploy {
    const RPC_METHOD: &'static str = Self::METHOD;
}

impl RpcClient for GetDeploy {
    const RPC_METHOD: &'static str = Self::METHOD;
}

impl RpcClient for SendDeploy {
    const RPC_METHOD: &'static str = PutDeploy::METHOD;
}

impl RpcClient for Transfer {
    const RPC_METHOD: &'static str = PutDeploy::METHOD;
}

/// Result for "chain_get_block" RPC response.
#[derive(Serialize, Deserialize, Debug)]
pub struct ListDeploysResult {
    /// The RPC API version.
    pub api_version: ProtocolVersion,
    /// The deploy hashes of the block, if found.
    pub deploy_hashes: Option<Vec<DeployHash>>,
    /// The transfer deploy hashes of the block, if found.
    pub transfer_hashes: Option<Vec<DeployHash>>,
}

impl From<GetBlockResult> for ListDeploysResult {
    fn from(get_block_result: GetBlockResult) -> Self {
        ListDeploysResult {
            api_version: get_block_result.api_version,
            deploy_hashes: get_block_result
                .block
                .as_ref()
                .map(|block| block.deploy_hashes().clone()),
            transfer_hashes: get_block_result
                .block
                .as_ref()
                .map(|block| block.transfer_hashes().clone()),
        }
    }
}

/// Creates a `Write` trait object respective to the path value passed.  A `File` is returned if
/// `maybe_path` is `Some`.  If `maybe_path` is `None`, a `Stdout` or `Sink` is returned; `Sink` for
/// test configuration to avoid cluttering test output.
pub(super) fn output_or_stdout(maybe_path: Option<&str>) -> io::Result<Box<dyn Write>> {
    match maybe_path {
        Some(output_path) => File::create(&output_path).map(|file| {
            let write: Box<dyn Write> = Box::new(file);
            write
        }),
        None => Ok(if cfg!(test) {
            Box::new(io::sink())
        } else {
            Box::new(io::stdout())
        }),
    }
}

/// `DeployParams` are used as a helper to construct a `Deploy` with
/// `DeployExt::with_payment_and_session`.
pub struct DeployParams {
    /// The secret key for this `Deploy`.
    pub secret_key: SecretKey,

    /// The creation timestamp of this `Deploy`.
    pub timestamp: Timestamp,

    /// The time to live for this `Deploy`.
    pub ttl: TimeDiff,

    /// The gas price for this `Deploy`.
    pub gas_price: u64,

    /// A list of other `Deploy`s (hashes) that this `Deploy` depends upon.
    pub dependencies: Vec<DeployHash>,

    /// The name of the chain this `Deploy` will be considered for inclusion in.
    pub chain_name: String,
}

/// An extension trait that adds some client-specific functionality to `Deploy`.
pub(super) trait DeployExt {
    /// Constructs a `Deploy`.
    fn with_payment_and_session(
        params: DeployParams,
        payment: ExecutableDeployItem,
        session: ExecutableDeployItem,
    ) -> Result<Deploy>;

    /// Constructs a transfer `Deploy`.
    fn new_transfer(
        amount: U512,
        source_purse: Option<URef>,
        target: TransferTarget,
        transfer_id: u64,
        params: DeployParams,
        payment: ExecutableDeployItem,
    ) -> Result<Deploy>;

    /// Writes the `Deploy` to `output`.
    fn write_deploy<W>(&self, output: W) -> Result<()>
    where
        W: Write;

    /// Reads a `Deploy` from the `input`.
    fn read_deploy<R>(input: R) -> Result<Deploy>
    where
        R: Read;

    /// Reads a `Deploy` from the reader at `input`, signs it, then writes it back to `output`.
    fn sign_and_write_deploy<R, W>(input: R, secret_key: SecretKey, output: W) -> Result<()>
    where
        R: Read,
        W: Write;
}

impl DeployExt for Deploy {
    fn with_payment_and_session(
        params: DeployParams,
        payment: ExecutableDeployItem,
        session: ExecutableDeployItem,
    ) -> Result<Deploy> {
        let DeployParams {
            timestamp,
            ttl,
            gas_price,
            dependencies,
            chain_name,
            secret_key,
        } = params;

        let deploy = Deploy::new(
            timestamp,
            ttl,
            gas_price,
            dependencies,
            chain_name,
            payment,
            session,
            &secret_key,
        );
        deploy.is_valid_size(MAX_SERIALIZED_SIZE)?;
        Ok(deploy)
    }

    fn new_transfer(
        amount: U512,
        source_purse: Option<URef>,
        target: TransferTarget,
        transfer_id: u64,
        params: DeployParams,
        payment: ExecutableDeployItem,
    ) -> Result<Deploy> {
        const TRANSFER_ARG_AMOUNT: &str = "amount";
        const TRANSFER_ARG_SOURCE: &str = "source";
        const TRANSFER_ARG_TARGET: &str = "target";
        const TRANSFER_ARG_ID: &str = "id";

        let mut transfer_args = RuntimeArgs::new();
        transfer_args.insert(TRANSFER_ARG_AMOUNT, amount)?;
        if let Some(source_purse) = source_purse {
            transfer_args.insert(TRANSFER_ARG_SOURCE, source_purse)?;
        }
        match target {
            TransferTarget::Account(target_account) => {
                let target_account_hash = target_account.to_account_hash().value();
                transfer_args.insert(TRANSFER_ARG_TARGET, target_account_hash)?;
            }
        }
        let maybe_transfer_id = Some(transfer_id);
        transfer_args.insert(TRANSFER_ARG_ID, maybe_transfer_id)?;
        let session = ExecutableDeployItem::Transfer {
            args: transfer_args,
        };
        Deploy::with_payment_and_session(params, payment, session)
    }

    fn write_deploy<W>(&self, mut output: W) -> Result<()>
    where
        W: Write,
    {
        let content = serde_json::to_string_pretty(self)?;
        output
            .write_all(content.as_bytes())
            .map_err(|error| Error::IoError {
                context: "unable to write deploy".to_owned(),
                error,
            })
    }

    fn read_deploy<R>(input: R) -> Result<Deploy>
    where
        R: Read,
    {
        let reader = BufReader::new(input);
        let deploy: Deploy = serde_json::from_reader(reader)?;
        deploy.is_valid_size(MAX_SERIALIZED_SIZE)?;
        Ok(deploy)
    }

    fn sign_and_write_deploy<R, W>(input: R, secret_key: SecretKey, output: W) -> Result<()>
    where
        R: Read,
        W: Write,
    {
        let mut deploy = Deploy::read_deploy(input)?;
        deploy.sign(&secret_key);
        deploy.is_valid_size(MAX_SERIALIZED_SIZE)?;
        deploy.write_deploy(output)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryInto;

    use casper_node::{crypto::AsymmetricKeyExt, types::ExcessiveSizeDeployError};

    use super::*;
    use crate::{DeployStrParams, PaymentStrParams, SessionStrParams};

    const PKG_HASH: &str = "09dcee4b212cfd53642ab323fbef07dafafc6f945a80a00147f62910a915c4e6";
    const ENTRYPOINT: &str = "entrypoint";
    const VERSION: &str = "0.1.0";
    const SAMPLE_DEPLOY: &str = r#"{
      "hash": "4858bbd79ab7b825244c4e6959cbcd588a05608168ef36518bc6590937191d55",
      "header": {
        "account": "01f60bce2bb1059c41910eac1e7ee6c3ef4c8fcc63a901eb9603c1524cadfb0c18",
        "timestamp": "2021-01-19T01:18:19.120Z",
        "ttl": "10s",
        "gas_price": 1,
        "body_hash": "95f2f2358c4864f01f8b073ae6f5ae67baeaf7747fc0799d0078743c513bc1de",
        "dependencies": [
          "be5fdeea0240e999e376f8ecbce1bd4fd9336f58dae4a5842558a4da6ad35aa8",
          "168d7ea9c88e76b3eef72759f2a7af24663cc871a469c7ba1387ca479e82fb41"
        ],
        "chain_name": "casper-test-chain-name-1"
      },
      "payment": {
        "StoredVersionedContractByHash": {
          "hash": "09dcee4b212cfd53642ab323fbef07dafafc6f945a80a00147f62910a915c4e6",
          "version": null,
          "entry_point": "entrypoint",
          "args": [
            [
              "name_01",
              {
                "cl_type": "Bool",
                "bytes": "00",
                "parsed": false
              }
            ],
            [
              "name_02",
              {
                "cl_type": "I32",
                "bytes": "2a000000",
                "parsed": 42
              }
            ]
          ]
        }
      },
      "session": {
        "StoredVersionedContractByHash": {
          "hash": "09dcee4b212cfd53642ab323fbef07dafafc6f945a80a00147f62910a915c4e6",
          "version": null,
          "entry_point": "entrypoint",
          "args": [
            [
              "name_01",
              {
                "cl_type": "Bool",
                "bytes": "00",
                "parsed": false
              }
            ],
            [
              "name_02",
              {
                "cl_type": "I32",
                "bytes": "2a000000",
                "parsed": 42
              }
            ]
          ]
        }
      },
      "approvals": [
        {
          "signer": "01f60bce2bb1059c41910eac1e7ee6c3ef4c8fcc63a901eb9603c1524cadfb0c18",
          "signature": "010f538ef188770cdbf608bc2d7aa9460108b419b2b629f5e0714204a7f29149809a1d52776b0c514e3320494fdf6f9e9747f06f2c14ddf6f924ce218148e2840a"
        },
        {
          "signer": "01e67d6e56ae07eca98b07ecec8cfbe826b4d5bc51f3a86590c0882cdafbd72fcc",
          "signature": "01c4f58d7f6145c1e4397efce766149cde5450cbe74991269161e5e1f30a397e6bc4c484f3c72a645cefd42c55cfde0294bfd91de55ca977798c3c8d2a7e43a40c"
        }
      ]
    }"#;
    // This intentionally has awkward indentation. It was formatted with serde::to_string_pretty.
    // It is used in serialization tests.
    const CHECK_SUMMED_SAMPLE_DEPLOY: &str = r#"{
  "hash": "4858bBd79ab7B825244C4E6959CbCd588a05608168ef36518Bc6590937191D55",
  "header": {
    "account": "01f60BCE2bb1059C41910eac1e7EE6C3ef4C8fCC63a901eb9603c1524CAdFb0C18",
    "timestamp": "2021-01-19T01:18:19.120Z",
    "ttl": "10s",
    "gas_price": 1,
    "body_hash": "95f2f2358C4864f01F8b073aE6f5aE67baeAF7747fc0799D0078743C513Bc1De",
    "dependencies": [
      "Be5fDEEA0240E999E376F8ecbCe1Bd4Fd9336F58daE4a5842558a4dA6aD35aA8",
      "168d7Ea9c88E76B3eEF72759F2A7af24663CC871A469C7bA1387CA479e82fB41"
    ],
    "chain_name": "casper-test-chain-name-1"
  },
  "payment": {
    "StoredVersionedContractByHash": {
      "hash": "09DCeE4b212cfd53642AB323FbEf07daFAFC6F945A80a00147f62910A915c4E6",
      "version": null,
      "entry_point": "entrypoint",
      "args": [
        [
          "name_01",
          {
            "cl_type": "Bool",
            "bytes": "00",
            "parsed": false
          }
        ],
        [
          "name_02",
          {
            "cl_type": "I32",
            "bytes": "2a000000",
            "parsed": 42
          }
        ]
      ]
    }
  },
  "session": {
    "StoredVersionedContractByHash": {
      "hash": "09DCeE4b212cfd53642AB323FbEf07daFAFC6F945A80a00147f62910A915c4E6",
      "version": null,
      "entry_point": "entrypoint",
      "args": [
        [
          "name_01",
          {
            "cl_type": "Bool",
            "bytes": "00",
            "parsed": false
          }
        ],
        [
          "name_02",
          {
            "cl_type": "I32",
            "bytes": "2a000000",
            "parsed": 42
          }
        ]
      ]
    }
  },
  "approvals": [
    {
      "signer": "01f60BCE2bb1059C41910eac1e7EE6C3ef4C8fCC63a901eb9603c1524CAdFb0C18",
      "signature": "010f538ef188770CdBF608BC2d7aa9460108B419b2b629f5E0714204A7f29149809A1d52776B0c514e3320494fDf6f9e9747f06f2c14dDF6f924CE218148E2840A"
    },
    {
      "signer": "01e67D6E56AE07ECA98b07ecEC8cfBe826B4D5bC51F3a86590C0882cdAfbD72FCC",
      "signature": "01c4F58D7f6145C1e4397efCe766149CdE5450cBe74991269161e5E1f30a397e6Bc4c484F3C72a645cEfD42c55CfdE0294Bfd91dE55Ca977798c3C8D2A7e43a40c"
    }
  ]
}"#;

    #[derive(Debug)]
    struct ErrWrapper(pub Error);

    impl PartialEq for ErrWrapper {
        fn eq(&self, other: &ErrWrapper) -> bool {
            format!("{:?}", self.0) == format!("{:?}", other.0)
        }
    }

    pub fn deploy_params() -> DeployStrParams<'static> {
        DeployStrParams {
            secret_key: "../resources/local/secret_keys/node-1.pem",
            ttl: "10s",
            chain_name: "casper-test-chain-name-1",
            gas_price: "1",
            dependencies: vec![
                "be5fdeea0240e999e376f8ecbce1bd4fd9336f58dae4a5842558a4da6ad35aa8",
                "168d7ea9c88e76b3eef72759f2a7af24663cc871a469c7ba1387ca479e82fb41",
            ],
            ..Default::default()
        }
    }

    fn args_simple() -> Vec<&'static str> {
        vec!["name_01:bool='false'", "name_02:i32='42'"]
    }

    #[test]
    fn should_create_deploy() {
        let deploy_params = deploy_params();
        let payment_params =
            PaymentStrParams::with_package_hash(PKG_HASH, VERSION, ENTRYPOINT, args_simple(), "");
        let session_params =
            SessionStrParams::with_package_hash(PKG_HASH, VERSION, ENTRYPOINT, args_simple(), "");

        let mut output = Vec::new();

        let deploy = Deploy::with_payment_and_session(
            deploy_params.try_into().unwrap(),
            payment_params.try_into().unwrap(),
            session_params.try_into().unwrap(),
        )
        .unwrap();
        deploy.write_deploy(&mut output).unwrap();

        // The test output can be used to generate data for SAMPLE_DEPLOY:
        // let secret_key = SecretKey::generate_ed25519().unwrap();
        // deploy.sign(&secret_key, &mut casper_node::new_rng());
        // println!("{}", serde_json::to_string_pretty(&deploy).unwrap());

        let result = String::from_utf8(output).unwrap();

        let expected = Deploy::read_deploy(SAMPLE_DEPLOY.as_bytes()).unwrap();
        let actual = Deploy::read_deploy(result.as_bytes()).unwrap();

        assert_eq!(expected.header().account(), actual.header().account());
        assert_eq!(expected.header().ttl(), actual.header().ttl());
        assert_eq!(expected.header().gas_price(), actual.header().gas_price());
        assert_eq!(expected.header().body_hash(), actual.header().body_hash());
        assert_eq!(expected.payment(), actual.payment());
        assert_eq!(expected.session(), actual.session());
    }

    #[test]
    fn should_fail_to_create_large_deploy() {
        let deploy_params = deploy_params();
        let payment_params =
            PaymentStrParams::with_package_hash(PKG_HASH, VERSION, ENTRYPOINT, args_simple(), "");
        // Create a string arg of 1048576 letter 'a's to ensure the deploy is greater than 1048576
        // bytes.
        let large_args_simple = format!("name_01:string='{:a<1048576}'", "");

        let session_params = SessionStrParams::with_package_hash(
            PKG_HASH,
            VERSION,
            ENTRYPOINT,
            vec![large_args_simple.as_str()],
            "",
        );

        match Deploy::with_payment_and_session(
            deploy_params.try_into().unwrap(),
            payment_params.try_into().unwrap(),
            session_params.try_into().unwrap(),
        ) {
            Err(Error::DeploySizeTooLarge(ExcessiveSizeDeployError {
                max_deploy_size,
                actual_deploy_size,
            })) => {
                assert_eq!(max_deploy_size, MAX_SERIALIZED_SIZE);
                assert!(actual_deploy_size > MAX_SERIALIZED_SIZE as usize);
            }
            Err(error) => panic!("unexpected error: {}", error),
            Ok(_) => panic!("failed to error while creating an excessively large deploy"),
        }
    }

    #[test]
    fn should_read_deploy() {
        let bytes = SAMPLE_DEPLOY.as_bytes();
        assert_eq!(
            Deploy::read_deploy(bytes).map(|_| ()).map_err(ErrWrapper),
            Ok(())
        );
    }

    #[test]
    fn should_round_trip_check_summed_sample_deploy_json() {
        let bytes = CHECK_SUMMED_SAMPLE_DEPLOY.as_bytes();
        let reader = BufReader::new(bytes);
        let deploy: Deploy = serde_json::from_reader(reader).unwrap();
        let casper_case_json_string = serde_json::to_string_pretty(&deploy).unwrap();
        assert_eq!(CHECK_SUMMED_SAMPLE_DEPLOY, casper_case_json_string);
    }

    #[test]
    fn should_sign_deploy() {
        let bytes = SAMPLE_DEPLOY.as_bytes();
        let mut deploy = Deploy::read_deploy(bytes).unwrap();
        deploy
            .is_valid()
            .unwrap_or_else(|error| panic!("{} - {:#?}", error, deploy));
        assert_eq!(
            deploy.approvals().len(),
            2,
            "Sample deploy should have 2 approvals."
        );

        let mut result = Vec::new();
        let secret_key = SecretKey::generate_ed25519().unwrap();
        Deploy::sign_and_write_deploy(bytes, secret_key, &mut result).unwrap();
        let signed_deploy = Deploy::read_deploy(&result[..]).unwrap();

        assert_eq!(
            signed_deploy.approvals().len(),
            deploy.approvals().len() + 1,
            "deploy should be is_valid() because it has been signed {:#?}",
            signed_deploy
        );
    }
}
