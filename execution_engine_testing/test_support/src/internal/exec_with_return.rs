use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use casper_execution_engine::{
    core::{
        engine_state,
        engine_state::{
            executable_deploy_item::ExecutableDeployItem, execution_effect::ExecutionEffect,
            EngineConfig,
        },
        execution::{self, AddressGenerator},
        runtime::{self, Runtime},
        runtime_context::RuntimeContext,
    },
    shared::{gas::Gas, gas_counter::GasCounter, newtypes::CorrelationId, wasm_prep::Preprocessor},
    storage::{global_state::StateProvider, protocol_data::ProtocolData},
};
use casper_types::{
    account::AccountHash, bytesrepr::FromBytes, system::CallStackElement, BlockTime, CLTyped,
    DeployHash, EntryPointType, Key, Phase, ProtocolVersion, RuntimeArgs, URef, U512,
};

use crate::internal::{utils, WasmTestBuilder, DEFAULT_WASM_CONFIG};

use super::DEFAULT_SYSTEM_CONFIG;

/// This function allows executing the contract stored in the given `wasm_file`, while capturing the
/// output. It is essentially the same functionality as `Executor::exec`, but the return value of
/// the contract is returned along with the effects. The purpose of this function is to test
/// installer contracts used in the new genesis process.
#[allow(clippy::too_many_arguments)]
pub fn exec<S, T>(
    config: EngineConfig,
    builder: &mut WasmTestBuilder<S>,
    address: AccountHash,
    wasm_file: &str,
    block_time: u64,
    deploy_hash: DeployHash,
    entry_point_name: &str,
    args: RuntimeArgs,
    extra_urefs: Vec<URef>,
    call_stack: Vec<CallStackElement>,
) -> Option<(T, Vec<URef>, ExecutionEffect)>
where
    S: StateProvider,
    engine_state::Error: From<S::Error>,
    S::Error: Into<execution::Error>,
    T: FromBytes + CLTyped,
{
    let prestate = builder.get_post_state_hash();

    let tracking_copy = Rc::new(RefCell::new(
        builder
            .get_engine_state()
            .tracking_copy(prestate)
            .unwrap()
            .expect("should be able to checkout tracking copy"),
    ));

    let phase = Phase::Session;
    let address_generator = {
        let address_generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
        Rc::new(RefCell::new(address_generator))
    };
    let transfer_address_generator = {
        let address_generator = AddressGenerator::new(deploy_hash.as_bytes(), phase);
        Rc::new(RefCell::new(address_generator))
    };
    let fn_store_id = {
        let fn_store_id = AddressGenerator::new(deploy_hash.as_bytes(), phase);
        Rc::new(RefCell::new(fn_store_id))
    };
    let gas_counter = GasCounter::new(Gas::new(U512::from(std::u64::MAX)), Gas::default());
    let protocol_version = ProtocolVersion::V1_0_0;
    let correlation_id = CorrelationId::new();
    let base_key = Key::Account(address);

    let account = builder.get_account(address).expect("should find account");

    let mut named_keys = account.named_keys().clone();

    let access_rights = {
        let mut ret = runtime::extract_access_rights_from_keys(named_keys.values().cloned());
        let extras = runtime::extract_access_rights_from_urefs(extra_urefs.into_iter());
        ret.extend(extras.into_iter());
        ret
    };

    let protocol_data = {
        let mint = builder.get_mint_contract_hash();
        let handle_payment = builder.get_mint_contract_hash();
        let standard_payment = builder.get_standard_payment_contract_hash();
        let auction = builder.get_auction_contract_hash();
        ProtocolData::new(
            *DEFAULT_WASM_CONFIG,
            *DEFAULT_SYSTEM_CONFIG,
            mint,
            handle_payment,
            standard_payment,
            auction,
        )
    };

    let transfers = Vec::default();

    let context = RuntimeContext::new(
        Rc::clone(&tracking_copy),
        EntryPointType::Session, // Is it always?
        &mut named_keys,
        access_rights,
        args,
        BTreeSet::new(),
        &account,
        base_key,
        BlockTime::new(block_time),
        deploy_hash,
        gas_counter,
        fn_store_id,
        address_generator,
        transfer_address_generator,
        protocol_version,
        correlation_id,
        phase,
        protocol_data,
        transfers,
    );

    let wasm_bytes = utils::read_wasm_file_bytes(wasm_file);
    let deploy_item = ExecutableDeployItem::ModuleBytes {
        module_bytes: wasm_bytes.into(),
        args: RuntimeArgs::new(),
    };

    let wasm_config = *DEFAULT_WASM_CONFIG;

    let preprocessor = Preprocessor::new(wasm_config);
    let parity_module = deploy_item
        .get_deploy_metadata(
            tracking_copy,
            &account,
            correlation_id,
            &preprocessor,
            &protocol_version,
            &protocol_data,
            phase,
        )
        .expect("should get wasm module");

    let module = parity_module.take_module();

    let (instance, memory) =
        runtime::instance_and_memory(module.clone(), protocol_version, &wasm_config)
            .expect("should be able to make wasm instance from module");

    let mut runtime = Runtime::new(
        config,
        Default::default(),
        memory,
        module,
        context,
        call_stack,
    );

    match instance.invoke_export(entry_point_name, &[], &mut runtime) {
        Ok(_) => None,
        Err(e) => {
            if let Some(host_error) = e.as_host_error() {
                // `ret` Trap is a success; downcast and attempt to extract result
                let downcasted_error = host_error.downcast_ref::<execution::Error>().unwrap();
                match downcasted_error {
                    execution::Error::Ret(ref ret_urefs) => {
                        let effect = runtime.context().effect();
                        let urefs = ret_urefs.clone();

                        let value: T = runtime
                            .take_host_buffer()
                            .expect("should have return value in the host_buffer")
                            .into_t()
                            .expect("should deserialize return value");

                        Some((value, urefs, effect))
                    }

                    _ => None,
                }
            } else {
                None
            }
        }
    }
}
