use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use casper_execution_engine::core::{
    execution::AddressGenerator,
    runtime_context::mocks::{mock_account, mock_runtime_context},
};
use casper_types::{account::AccountHash, contracts::NamedKeys, Phase};

fn runtime_bench(c: &mut Criterion) {
    let access_rights = Default::default();

    let deploy_hash = [1u8; 32];
    let (base_key, account) = mock_account(AccountHash::new([0u8; 32]));

    let mut named_keys = NamedKeys::new();
    let uref_address_generator = AddressGenerator::new(&deploy_hash, Phase::Session);
    let hash_address_generator = AddressGenerator::new(&deploy_hash, Phase::Session);
    let transfer_address_generator = AddressGenerator::new(&deploy_hash, Phase::Session);
    let mut runtime_context = mock_runtime_context(
        &account,
        base_key,
        &mut named_keys,
        access_rights,
        hash_address_generator,
        uref_address_generator,
        transfer_address_generator,
    );

    let gas_arg = u16::MAX as u32;

    c.bench_with_input(
        BenchmarkId::new("gas_charge", gas_arg),
        &gas_arg,
        |b, &s| {
            b.iter(|| runtime_context.charge_gas(s.into()).unwrap());
        },
    );
}

criterion_group!(benches, runtime_bench);
criterion_main!(benches);
