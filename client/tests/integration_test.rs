use std::{convert::Infallible, fs, net::SocketAddr, sync::Arc, time::Duration};

use futures::{channel::oneshot, future};
use hyper::{Body, Response, Server};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::{sync::Mutex, task::JoinHandle};
use tower::builder::ServiceBuilder;
use warp::{Filter, Rejection};
use warp_json_rpc::Builder;

use casper_node::crypto::Error as CryptoError;
use hex::FromHexError;

use casper_client::{DeployStrParams, Error, PaymentStrParams, SessionStrParams};
use casper_node::rpcs::{
    account::{PutDeploy, PutDeployParams},
    chain::{GetStateRootHash, GetStateRootHashParams},
    info::{GetDeploy, GetDeployParams},
    state::{GetBalance, GetBalanceParams},
    RpcWithOptionalParams, RpcWithParams,
};

const VALID_PURSE_UREF: &str =
    "uref-0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20-007";
const VALID_STATE_ROOT_HASH: &str =
    "55db08058acb54c295b115cbd9b282eb2862e76d5bb8493bb80c0598a50a12a5";

const DEFAULT_RATE_LIMIT: u64 = 1;
const DEFAULT_RATE_PER: Duration = Duration::from_secs(1);

fn test_filter<P>(
    method: &'static str,
) -> impl Filter<Extract = (Response<Body>,), Error = Rejection> + Copy
where
    for<'de> P: Deserialize<'de> + Send,
{
    warp_json_rpc::filters::json_rpc()
        .and(warp_json_rpc::filters::method(method))
        .and(warp_json_rpc::filters::params::<P>())
        .map(|builder: Builder, _params: P| builder.success(()).unwrap())
}

fn test_filter_without_params(
    method: &'static str,
) -> impl Filter<Extract = (Response<Body>,), Error = Rejection> + Copy {
    warp_json_rpc::filters::json_rpc()
        .and(warp_json_rpc::filters::method(method))
        .map(|builder: Builder| builder.success(()).unwrap())
}

type ServerJoiner = Option<Arc<Mutex<JoinHandle<Result<(), hyper::Error>>>>>;

struct MockServerHandle {
    graceful_shutdown: Option<oneshot::Sender<()>>,
    server_joiner: ServerJoiner,
    address: SocketAddr,
}

trait Captures<'a> {}
impl<'a, T: ?Sized> Captures<'a> for T {}

impl MockServerHandle {
    fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Will spawn a server on localhost and respond to JSON-RPC requests that successfully
    /// deserialize as `P`.
    fn spawn<P>(method: &'static str) -> Self
    where
        P: 'static,
        for<'de> P: Deserialize<'de> + Send,
    {
        Self::spawn_with_filter(
            test_filter::<P>(method),
            DEFAULT_RATE_LIMIT,
            DEFAULT_RATE_PER,
        )
    }

    /// Will spawn a server on localhost and respond to JSON-RPC requests that don't take
    /// parameters.
    fn spawn_without_params(method: &'static str) -> Self {
        Self::spawn_with_filter(
            test_filter_without_params(method),
            DEFAULT_RATE_LIMIT,
            DEFAULT_RATE_PER,
        )
    }

    fn spawn_with_filter<F>(filter: F, rate: u64, per: Duration) -> Self
    where
        F: Filter<Extract = (Response<Body>,), Error = Rejection> + Send + Sync + 'static + Copy,
    {
        let service = warp_json_rpc::service(filter);

        let make_svc =
            hyper::service::make_service_fn(move |_| future::ok::<_, Infallible>(service.clone()));

        let make_svc = ServiceBuilder::new()
            .rate_limit(rate, per)
            .service(make_svc);

        let builder = Server::try_bind(&([127, 0, 0, 1], 0).into()).unwrap();
        let (graceful_shutdown, shutdown_receiver) = oneshot::channel::<()>();
        let graceful_shutdown = Some(graceful_shutdown);
        let server = builder.serve(make_svc);
        let address = server.local_addr();
        let server_with_shutdown = server.with_graceful_shutdown(async {
            shutdown_receiver.await.ok();
        });
        let server_joiner = tokio::spawn(server_with_shutdown);
        let server_joiner = Some(Arc::new(Mutex::new(server_joiner)));
        MockServerHandle {
            graceful_shutdown,
            server_joiner,
            address,
        }
    }

    async fn get_balance(&self, state_root_hash: &str, purse_uref: &str) -> Result<(), ErrWrapper> {
        casper_client::get_balance("1", &self.url(), 0, state_root_hash, purse_uref)
            .await
            .map(|_| ())
            .map_err(ErrWrapper)
    }

    async fn get_deploy(&self, deploy_hash: &str) -> Result<(), ErrWrapper> {
        casper_client::get_deploy("1", &self.url(), 0, deploy_hash)
            .await
            .map(|_| ())
            .map_err(ErrWrapper)
    }

    async fn get_state_root_hash(&self, maybe_block_id: &str) -> Result<(), ErrWrapper> {
        casper_client::get_state_root_hash("1", &self.url(), 0, maybe_block_id)
            .await
            .map(|_| ())
            .map_err(ErrWrapper)
    }

    async fn get_block(&self, maybe_block_id: &str) -> Result<(), ErrWrapper> {
        casper_client::get_block("1", &self.url(), 0, maybe_block_id)
            .await
            .map(|_| ())
            .map_err(ErrWrapper)
    }

    async fn get_item(
        &self,
        state_root_hash: &str,
        key: &str,
        path: &str,
    ) -> Result<(), ErrWrapper> {
        casper_client::get_item("1", &self.url(), 0, state_root_hash, key, path)
            .await
            .map(|_| ())
            .map_err(ErrWrapper)
    }

    async fn transfer(
        &self,
        amount: &str,
        maybe_target_account: &str,
        deploy_params: DeployStrParams<'_>,
        payment_params: PaymentStrParams<'_>,
    ) -> Result<(), ErrWrapper> {
        casper_client::transfer(
            "1",
            &self.url(),
            0,
            amount,
            maybe_target_account,
            "2",
            deploy_params,
            payment_params,
        )
        .await
        .map(|_| ())
        .map_err(ErrWrapper)
    }

    async fn put_deploy(
        &self,
        deploy_params: DeployStrParams<'_>,
        session_params: SessionStrParams<'_>,
        payment_params: PaymentStrParams<'_>,
    ) -> Result<(), ErrWrapper> {
        casper_client::put_deploy(
            "1",
            &self.url(),
            0,
            deploy_params,
            session_params,
            payment_params,
        )
        .await
        .map(|_| ())
        .map_err(ErrWrapper)
    }

    async fn send_deploy_file(&self, input_path: &str) -> Result<(), ErrWrapper> {
        casper_client::send_deploy_file("1", &self.url(), 0, input_path)
            .await
            .map(|_| ())
            .map_err(ErrWrapper)
    }

    async fn get_auction_info(&self, maybe_block_id: &str) -> Result<(), ErrWrapper> {
        casper_client::get_auction_info("1", &self.url(), 0, maybe_block_id)
            .await
            .map(|_| ())
            .map_err(ErrWrapper)
    }
}

impl Drop for MockServerHandle {
    fn drop(&mut self) {
        let _ = self.graceful_shutdown.take().unwrap().send(());
        let joiner = self.server_joiner.take().unwrap();
        futures::executor::block_on(async {
            let join = &mut *joiner.lock().await;
            let _ = join.await;
        });
    }
}

#[derive(Debug)]
struct ErrWrapper(pub Error);

impl PartialEq for ErrWrapper {
    fn eq(&self, other: &ErrWrapper) -> bool {
        format!("{:?}", self.0) == format!("{:?}", other.0)
    }
}

impl From<Error> for ErrWrapper {
    fn from(error: Error) -> Self {
        ErrWrapper(error)
    }
}

mod deploy_params {
    use super::*;

    pub fn test_data_valid() -> DeployStrParams<'static> {
        DeployStrParams {
            secret_key: "../resources/local/secret_keys/node-1.pem",
            ttl: "10s",
            chain_name: "casper-test-chain-name-1",
            gas_price: "1",
            ..Default::default()
        }
    }
}

/// Sample data creation methods for PaymentStrParams
mod payment_params {
    use super::*;

    const NAME: &str = "name";
    const ENTRYPOINT: &str = "entrypoint";

    fn args_simple() -> Vec<&'static str> {
        vec!["name_01:bool='false'", "name_02:i32='42'"]
    }

    pub fn test_data_with_name() -> PaymentStrParams<'static> {
        PaymentStrParams::with_name(NAME, ENTRYPOINT, args_simple(), "")
    }
}

/// Sample data creation methods for SessionStrParams
mod session_params {
    use super::*;

    const PKG_HASH: &str = "09dcee4b212cfd53642ab323fbef07dafafc6f945a80a00147f62910a915c4e6";
    const ENTRYPOINT: &str = "entrypoint";
    const VERSION: &str = "0.1.0";

    fn args_simple() -> Vec<&'static str> {
        vec!["name_01:bool='false'", "name_02:i32='42'"]
    }

    pub fn test_data_with_package_hash() -> SessionStrParams<'static> {
        SessionStrParams::with_package_hash(PKG_HASH, VERSION, ENTRYPOINT, args_simple(), "")
    }
}

mod get_balance {
    use super::*;

    use casper_client::ValidateResponseError;
    use casper_types::URefFromStrError;

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_with_valid_arguments() {
        let server_handle = MockServerHandle::spawn::<GetBalanceParams>(GetBalance::METHOD);
        assert_eq!(
            server_handle
                .get_balance(VALID_STATE_ROOT_HASH, VALID_PURSE_UREF)
                .await,
            // NOTE: this "success" means that we then fail to validate the response, but that
            // is outside the scope of this test.
            // The MockServerHandle could support a pre-baked response, which should successfully
            // validate
            Err(
                Error::InvalidResponse(ValidateResponseError::ValidateResponseFailedToParse).into()
            )
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_empty_arguments() {
        let server_handle = MockServerHandle::spawn::<GetBalanceParams>(GetBalance::METHOD);
        assert_eq!(
            server_handle.get_balance("", "").await,
            Err(Error::CryptoError {
                context: "state_root_hash",
                error: CryptoError::FromHex(FromHexError::InvalidStringLength)
            }
            .into())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_empty_state_root_hash() {
        let server_handle = MockServerHandle::spawn::<GetBalanceParams>(GetBalance::METHOD);
        assert_eq!(
            server_handle.get_balance("", VALID_PURSE_UREF).await,
            Err(Error::CryptoError {
                context: "state_root_hash",
                error: CryptoError::FromHex(FromHexError::InvalidStringLength)
            }
            .into())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_empty_purse_uref() {
        let server_handle = MockServerHandle::spawn::<GetBalanceParams>(GetBalance::METHOD);
        assert_eq!(
            server_handle.get_balance(VALID_STATE_ROOT_HASH, "").await,
            Err(Error::FailedToParseURef("purse_uref", URefFromStrError::InvalidPrefix).into())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_bad_state_root_hash() {
        let server_handle = MockServerHandle::spawn::<GetBalanceParams>(GetBalance::METHOD);
        assert_eq!(
            server_handle
                .get_balance("deadbeef", VALID_PURSE_UREF)
                .await,
            Err(Error::CryptoError {
                context: "state_root_hash",
                error: CryptoError::FromHex(FromHexError::InvalidStringLength)
            }
            .into())
        );
    }
}

mod get_state_root_hash {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_with_valid_block_id() {
        let server_handle =
            MockServerHandle::spawn::<GetStateRootHashParams>(GetStateRootHash::METHOD);
        assert_eq!(
            server_handle
                .get_state_root_hash(
                    "7a073a340bb5e0ca60f4c1dbb3254fb0641da79cda7c5aeb5303efa74fcc9eb1",
                )
                .await,
            Ok(())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_with_empty_block_id() {
        let server_handle = MockServerHandle::spawn_without_params(GetStateRootHash::METHOD);
        assert_eq!(server_handle.get_state_root_hash("").await, Ok(()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_with_valid_block_height() {
        let server_handle = MockServerHandle::spawn_without_params(GetStateRootHash::METHOD);
        assert_eq!(server_handle.get_state_root_hash("1").await, Ok(()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_bad_block_id() {
        let server_handle = MockServerHandle::spawn_without_params(GetStateRootHash::METHOD);
        let input = "<not a real block id>";
        assert!(
            server_handle.get_state_root_hash(input).await.is_err(),
            "input '{}' should not parse to a valid block id",
            input
        );
    }
}

mod get_block {
    use casper_client::ValidateResponseError;
    use casper_node::rpcs::chain::{GetBlock, GetBlockParams};

    use super::*;

    // in this case, the error means that the request was sent successfully, but due to to the
    // mock implementation fails to validate

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_with_valid_block_hash() {
        let server_handle = MockServerHandle::spawn::<GetBlockParams>(GetBlock::METHOD);
        assert_eq!(
            server_handle.get_block(VALID_STATE_ROOT_HASH).await,
            Err(ErrWrapper(Error::InvalidResponse(
                ValidateResponseError::NoBlockInResponse
            )))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_with_valid_block_height() {
        let server_handle = MockServerHandle::spawn::<GetBlockParams>(GetBlock::METHOD);
        assert_eq!(
            server_handle.get_block("1").await,
            Err(ErrWrapper(Error::InvalidResponse(
                ValidateResponseError::NoBlockInResponse
            )))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_with_valid_empty_block_hash() {
        let server_handle = MockServerHandle::spawn_without_params(GetBlock::METHOD);
        assert_eq!(
            server_handle.get_block("").await,
            Err(ErrWrapper(Error::InvalidResponse(
                ValidateResponseError::NoBlockInResponse
            )))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_invalid_block_id() {
        let server_handle = MockServerHandle::spawn::<GetBlockParams>(GetBlock::METHOD);
        match server_handle.get_block("<not a valid hash>").await {
            Err(ErrWrapper(Error::FailedToParseInt("block_identifier", _))) => {}
            other => panic!("incorrect error returned from client {:?}", other),
        }
    }
}

mod get_item {
    use casper_client::ValidateResponseError;
    use casper_node::rpcs::state::{GetItem, GetItemParams};

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_with_valid_state_root_hash() {
        let server_handle = MockServerHandle::spawn::<GetItemParams>(GetItem::METHOD);

        // in this case, the error means that the request was sent successfully, but due to to the
        // mock implementation fails to validate

        assert_eq!(
            server_handle
                .get_item(VALID_STATE_ROOT_HASH, VALID_PURSE_UREF, "")
                .await,
            Err(
                Error::InvalidResponse(ValidateResponseError::ValidateResponseFailedToParse).into()
            )
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_invalid_state_root_hash() {
        let server_handle = MockServerHandle::spawn::<GetItemParams>(GetItem::METHOD);
        assert_eq!(
            server_handle
                .get_item("<invalid state root hash>", VALID_PURSE_UREF, "")
                .await,
            Err(Error::CryptoError {
                context: "state_root_hash",
                error: CryptoError::FromHex(FromHexError::OddLength)
            }
            .into())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_invalid_key() {
        let server_handle = MockServerHandle::spawn::<GetItemParams>(GetItem::METHOD);
        assert_eq!(
            server_handle
                .get_item(VALID_STATE_ROOT_HASH, "invalid key", "")
                .await,
            Err(Error::FailedToParseKey.into())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_empty_key() {
        let server_handle = MockServerHandle::spawn::<GetItemParams>(GetItem::METHOD);
        assert_eq!(
            server_handle
                .get_item("<invalid state root hash>", "", "")
                .await,
            Err(Error::CryptoError {
                context: "state_root_hash",
                error: CryptoError::FromHex(FromHexError::OddLength)
            }
            .into())
        );
    }
}

mod get_deploy {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_with_valid_hash() {
        let server_handle = MockServerHandle::spawn::<GetDeployParams>(GetDeploy::METHOD);
        assert_eq!(
            server_handle
                .get_deploy("09dcee4b212cfd53642ab323fbef07dafafc6f945a80a00147f62910a915c4e6")
                .await,
            Ok(())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_invalid_hash() {
        let server_handle = MockServerHandle::spawn::<GetDeployParams>(GetDeploy::METHOD);
        assert_eq!(
            server_handle.get_deploy("012345",).await,
            Err(Error::CryptoError {
                context: "deploy_hash",
                error: CryptoError::FromHex(FromHexError::InvalidStringLength)
            }
            .into())
        );
    }
}

mod get_auction_info {
    use super::*;

    use casper_node::rpcs::state::GetAuctionInfo;

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed() {
        let server_handle = MockServerHandle::spawn_without_params(GetAuctionInfo::METHOD);
        assert_eq!(server_handle.get_auction_info("").await, Ok(()));
    }
}

mod make_deploy {
    use super::*;

    #[test]
    fn should_succeed_for_stdout() {
        assert_eq!(
            casper_client::make_deploy(
                "",
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
    }

    #[test]
    fn should_succeed_for_file() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let file_path = temp_dir.path().join("test_deploy.json");
        assert_eq!(
            casper_client::make_deploy(
                file_path.to_str().unwrap(),
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
    }

    #[test]
    fn should_fail_with_existing_file() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let file_path = temp_dir.path().join("test_deploy.json");
        let contents = "contents of test file";
        fs::write(file_path.clone(), &contents)
            .unwrap_or_else(|err| panic!("Failed to create temp file with error: {}", err));

        assert_eq!(
            casper_client::make_deploy(
                file_path.to_str().unwrap(),
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Err(Error::FileAlreadyExists(file_path.clone()).into())
        );

        let contents_after_fail = fs::read_to_string(file_path).unwrap_or_else(|err| {
            panic!("Failed to read contents of test file with error: {}", err)
        });

        assert_eq!(contents, contents_after_fail);
    }

    #[test]
    fn should_succeed_with_existing_file_with_force() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let file_path = temp_dir.path().join("test_deploy.json");
        fs::write(file_path.clone(), "hi")
            .unwrap_or_else(|err| panic!("Failed to create temp file with error: {}", err));

        assert_eq!(
            casper_client::make_deploy(
                file_path.to_str().unwrap(),
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                true
            )
            .map_err(ErrWrapper),
            Ok(())
        );
    }
}

mod send_deploy {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn should_fail_with_bad_deploy_file_path() {
        let server_handle = MockServerHandle::spawn::<PutDeployParams>(PutDeploy::METHOD);
        if let Err(ErrWrapper(Error::IoError { context, .. })) =
            server_handle.send_deploy_file("<not a valid path>").await
        {
            assert_eq!(context, "unable to read input file \'<not a valid path>\'")
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed_for_file() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let file_path = temp_dir.path().join("test_send_deploy.json");
        assert_eq!(
            casper_client::make_deploy(
                file_path.to_str().unwrap(),
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
        let server_handle = MockServerHandle::spawn::<PutDeployParams>(PutDeploy::METHOD);
        assert_eq!(
            server_handle
                .send_deploy_file(file_path.to_str().unwrap())
                .await,
            Ok(())
        );
    }
}

mod sign_deploy {
    use super::*;

    #[test]
    fn should_succeed_for_file() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let unsigned_file_path = temp_dir.path().join("test_deploy.json");
        let signed_file_path = temp_dir.path().join("signed_test_deploy.json");
        assert_eq!(
            casper_client::make_deploy(
                unsigned_file_path.to_str().unwrap(),
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
        assert_eq!(
            casper_client::sign_deploy_file(
                unsigned_file_path.to_str().unwrap(),
                "../resources/local/secret_keys/node-1.pem",
                signed_file_path.to_str().unwrap(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
    }

    #[test]
    fn should_succeed_for_stdout() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let unsigned_file_path = temp_dir.path().join("test_deploy.json");
        assert_eq!(
            casper_client::make_deploy(
                unsigned_file_path.to_str().unwrap(),
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
        assert_eq!(
            casper_client::sign_deploy_file(
                unsigned_file_path.to_str().unwrap(),
                "../resources/local/secret_keys/node-1.pem",
                "",
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
    }

    #[test]
    fn should_fail_with_bad_secret_key_path() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let unsigned_file_path = temp_dir.path().join("test_deploy.json");
        assert_eq!(
            casper_client::make_deploy(
                unsigned_file_path.to_str().unwrap(),
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
        assert!(casper_client::sign_deploy_file(
            unsigned_file_path.to_str().unwrap(),
            "<this is not a path>",
            "",
            false
        )
        .is_err());
    }

    #[test]
    fn should_fail_with_existing_file() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let unsigned_file_path = temp_dir.path().join("test_deploy.json");
        let signed_file_path = temp_dir.path().join("signed_test_deploy.json");
        assert_eq!(
            casper_client::make_deploy(
                unsigned_file_path.to_str().unwrap(),
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
        assert_eq!(
            casper_client::sign_deploy_file(
                unsigned_file_path.to_str().unwrap(),
                "../resources/local/secret_keys/node-1.pem",
                signed_file_path.to_str().unwrap(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );

        let contents = fs::read_to_string(signed_file_path.clone())
            .unwrap_or_else(|err| panic!("Failed to read contents of file with error: {}", err));

        assert_eq!(
            casper_client::sign_deploy_file(
                unsigned_file_path.to_str().unwrap(),
                "../resources/local/secret_keys/node-1.pem",
                signed_file_path.to_str().unwrap(),
                false
            )
            .map_err(ErrWrapper),
            Err(Error::FileAlreadyExists(signed_file_path.clone()).into())
        );

        let contents_after_failure = fs::read_to_string(signed_file_path)
            .unwrap_or_else(|err| panic!("Failed to read contents of file with error: {}", err));

        assert_eq!(contents, contents_after_failure);
    }

    #[test]
    fn should_succeed_with_existing_file_with_force() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let unsigned_file_path = temp_dir.path().join("test_deploy.json");
        let signed_file_path = temp_dir.path().join("signed_test_deploy.json");
        assert_eq!(
            casper_client::make_deploy(
                unsigned_file_path.to_str().unwrap(),
                deploy_params::test_data_valid(),
                session_params::test_data_with_package_hash(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
        assert_eq!(
            casper_client::sign_deploy_file(
                unsigned_file_path.to_str().unwrap(),
                "../resources/local/secret_keys/node-1.pem",
                signed_file_path.to_str().unwrap(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
        assert_eq!(
            casper_client::sign_deploy_file(
                unsigned_file_path.to_str().unwrap(),
                "../resources/local/secret_keys/node-1.pem",
                signed_file_path.to_str().unwrap(),
                true
            )
            .map_err(ErrWrapper),
            Ok(())
        );
    }
}

mod make_transfer {
    use super::*;

    const AMOUNT: &str = "30000000000";
    const TARGET_ACCOUNT: &str =
        "01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TRANSFER_ID: &str = "1314";

    #[test]
    fn should_succeed_for_stdout() {
        assert_eq!(
            casper_client::make_transfer(
                "",
                AMOUNT,
                TARGET_ACCOUNT,
                TRANSFER_ID,
                deploy_params::test_data_valid(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
    }

    #[test]
    fn should_succeed_for_file() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let file_path = temp_dir.path().join("test_deploy.json");
        assert_eq!(
            casper_client::make_transfer(
                file_path.to_str().unwrap(),
                AMOUNT,
                TARGET_ACCOUNT,
                TRANSFER_ID,
                deploy_params::test_data_valid(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Ok(())
        );
    }

    #[test]
    fn should_fail_with_existing_file() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let file_path = temp_dir.path().join("test_deploy.json");
        let contents = "contents of test file";
        fs::write(file_path.clone(), &contents)
            .unwrap_or_else(|err| panic!("Failed to create temp file with error: {}", err));

        assert_eq!(
            casper_client::make_transfer(
                file_path.to_str().unwrap(),
                AMOUNT,
                TARGET_ACCOUNT,
                TRANSFER_ID,
                deploy_params::test_data_valid(),
                payment_params::test_data_with_name(),
                false
            )
            .map_err(ErrWrapper),
            Err(Error::FileAlreadyExists(file_path.clone()).into())
        );

        let contents_after_fail = fs::read_to_string(file_path)
            .unwrap_or_else(|err| panic!("Failed to read from temp file with error: {}", err));

        assert_eq!(contents, contents_after_fail);
    }

    #[test]
    fn should_succeed_with_existing_file_with_force() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create temp dir with error: {}", err));
        let file_path = temp_dir.path().join("test_deploy.json");
        fs::write(file_path.clone(), "hi")
            .unwrap_or_else(|err| panic!("Failed to create temp file with error: {}", err));

        assert_eq!(
            casper_client::make_transfer(
                file_path.to_str().unwrap(),
                AMOUNT,
                TARGET_ACCOUNT,
                TRANSFER_ID,
                deploy_params::test_data_valid(),
                payment_params::test_data_with_name(),
                true
            )
            .map_err(ErrWrapper),
            Ok(())
        );
    }
}

mod keygen_generate_files {
    use super::*;

    #[test]
    fn should_succeed_for_valid_args_ed25519() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create a temp dir with error: {}", err));
        let path = temp_dir.path().join("test-keygen-ed25519");
        let result = casper_client::keygen::generate_files(
            path.to_str().unwrap(),
            casper_client::keygen::ED25519,
            true,
        )
        .map_err(ErrWrapper);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn should_succeed_for_valid_args_secp256k1() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create a temp dir with error: {}", err));
        let path = temp_dir.path().join("test-keygen-secp256k1");
        let result = casper_client::keygen::generate_files(
            path.to_str().unwrap(),
            casper_client::keygen::SECP256K1,
            true,
        )
        .map_err(ErrWrapper);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn should_force_overwrite_when_set() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create a temp dir with error: {}", err));
        let path = temp_dir.path().join("test-keygen-force");
        let result = casper_client::keygen::generate_files(
            path.to_str().unwrap(),
            casper_client::keygen::SECP256K1,
            false,
        )
        .map_err(ErrWrapper);
        assert_eq!(result, Ok(()));
        let result = casper_client::keygen::generate_files(
            path.to_str().unwrap(),
            casper_client::keygen::SECP256K1,
            false,
        )
        .map_err(ErrWrapper);
        assert_eq!(
            result,
            Err(Error::FileAlreadyExists(path.join("secret_key.pem")).into())
        );
        let result = casper_client::keygen::generate_files(
            path.to_str().unwrap(),
            casper_client::keygen::SECP256K1,
            true,
        )
        .map_err(ErrWrapper);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn should_fail_for_invalid_algorithm() {
        let temp_dir = TempDir::new()
            .unwrap_or_else(|err| panic!("Failed to create a temp dir with error: {}", err));
        let path = temp_dir.path().join("test-keygen-invalid-algo");
        let result = casper_client::keygen::generate_files(
            path.to_str().unwrap(),
            "<not a valid algo>",
            true,
        )
        .map_err(ErrWrapper);
        assert_eq!(
            result,
            Err(Error::UnsupportedAlgorithm("<not a valid algo>".to_string()).into())
        );
    }

    #[test]
    fn should_fail_for_invalid_output_dir() {
        let path = "";
        let result =
            casper_client::keygen::generate_files(path, casper_client::keygen::ED25519, true)
                .map_err(ErrWrapper);
        assert_eq!(
            result,
            Err(Error::InvalidArgument(
                "generate_files",
                "empty output_dir provided, must be a valid path".to_string()
            )
            .into())
        );
    }
}

mod put_deploy {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn should_send_put_deploy() {
        let server_handle = MockServerHandle::spawn::<PutDeployParams>(PutDeploy::METHOD);
        assert_eq!(
            server_handle
                .put_deploy(
                    deploy_params::test_data_valid(),
                    session_params::test_data_with_package_hash(),
                    payment_params::test_data_with_name()
                )
                .await,
            Ok(())
        );
    }
}

mod rate_limit {
    use super::*;
    use casper_node::types::Timestamp;

    #[tokio::test(flavor = "multi_thread")]
    async fn client_should_should_be_rate_limited_to_approx_1_qps() {
        // Transfer uses PutDeployParams + PutDeploy
        let server_handle = Arc::new(MockServerHandle::spawn::<PutDeployParams>(
            PutDeploy::METHOD,
        ));

        let now = Timestamp::now();
        // Our default is 1 req/s, so this will hit the threshold
        for _ in 0..3u32 {
            let amount = "100";
            let maybe_target_account =
                "01522ef6c89038019cb7af05c340623804392dd2bb1f4dab5e4a9c3ab752fc0179";

            let server_handle = server_handle.clone();

            assert_eq!(
                server_handle
                    .transfer(
                        amount,
                        maybe_target_account,
                        deploy_params::test_data_valid(),
                        payment_params::test_data_with_name(),
                    )
                    .await,
                Ok(())
            );
        }

        let diff = now.elapsed();
        assert!(
            diff < Duration::from_millis(4000).into(),
            "Rate limiting of 1 qps for 3 sec took too long at {}ms",
            diff.millis()
        );
        assert!(
            diff > Duration::from_millis(2000).into(),
            "Rate limiting of 1 qps for 3 sec too fast took {}ms",
            diff.millis()
        );
    }
}

mod transfer {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn should_succeed() {
        // Transfer uses PutDeployParams + PutDeploy
        let server_handle = MockServerHandle::spawn::<PutDeployParams>(PutDeploy::METHOD);
        let amount = "100";
        let maybe_target_account =
            "01522ef6c89038019cb7af05c340623804392dd2bb1f4dab5e4a9c3ab752fc0179";
        assert_eq!(
            server_handle
                .transfer(
                    amount,
                    maybe_target_account,
                    deploy_params::test_data_valid(),
                    payment_params::test_data_with_name()
                )
                .await,
            Ok(())
        );
    }
}
