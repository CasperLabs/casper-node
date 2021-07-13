use std::collections::VecDeque;

use casper_types::{bytesrepr::FromBytes, CLTyped, CLValue, CLValueError, Key, TransferAddr};

use super::{error, execution_effect::ExecutionEffect, op::Op};
use crate::{
    shared::{
        additive_map::AdditiveMap, gas::Gas, motes::Motes, newtypes::CorrelationId,
        stored_value::StoredValue, transform::Transform,
    },
    storage::global_state::StateReader,
};

fn make_payment_error_effects(
    max_payment_cost: Motes,
    account_main_purse_balance: Motes,
    account_main_purse_balance_key: Key,
    proposer_main_purse_balance_key: Key,
) -> Result<ExecutionEffect, CLValueError> {
    let mut ops = AdditiveMap::new();
    let mut transforms = AdditiveMap::new();

    let new_balance = account_main_purse_balance - max_payment_cost;
    // from_t for U512 is assumed to never panic
    let new_balance_clvalue = CLValue::from_t(new_balance.value())?;
    let new_balance_value = StoredValue::CLValue(new_balance_clvalue);

    let account_main_purse_balance_normalize = account_main_purse_balance_key.normalize();
    let proposer_main_purse_balance_normalize = proposer_main_purse_balance_key.normalize();

    ops.insert(account_main_purse_balance_normalize, Op::Write);
    transforms.insert(
        account_main_purse_balance_normalize,
        Transform::Write(new_balance_value),
    );

    ops.insert(proposer_main_purse_balance_normalize, Op::Add);
    transforms.insert(
        proposer_main_purse_balance_normalize,
        Transform::AddUInt512(max_payment_cost.value()),
    );

    Ok(ExecutionEffect::new(ops, transforms))
}

#[derive(Clone, Debug)]
pub enum ExecutionResult {
    /// An error condition that happened during execution
    Failure {
        error: error::Error,
        execution_effect: ExecutionEffect,
        transfers: Vec<TransferAddr>,
        cost: Gas,
    },
    /// Execution was finished successfully
    Success {
        execution_effect: ExecutionEffect,
        transfers: Vec<TransferAddr>,
        cost: Gas,
    },
}

impl Default for ExecutionResult {
    fn default() -> Self {
        ExecutionResult::Success {
            execution_effect: ExecutionEffect::default(),
            transfers: Vec::default(),
            cost: Gas::default(),
        }
    }
}

/// A type alias that represents multiple execution results.
pub type ExecutionResults = VecDeque<ExecutionResult>;

pub enum ForcedTransferResult {
    /// Payment code ran out of gas during execution
    InsufficientPayment,
    /// Gas conversion overflow
    GasConversionOverflow,
    /// Payment code execution resulted in an error
    PaymentFailure,
}

impl ExecutionResult {
    /// Constructs [ExecutionResult::Failure] that has 0 cost and no effects.
    /// This is the case for failures that we can't (or don't want to) charge
    /// for, like `PreprocessingError` or `InvalidNonce`.
    pub fn precondition_failure(error: error::Error) -> ExecutionResult {
        ExecutionResult::Failure {
            error,
            execution_effect: Default::default(),
            transfers: Vec::default(),
            cost: Gas::default(),
        }
    }

    pub fn is_success(&self) -> bool {
        match self {
            ExecutionResult::Failure { .. } => false,
            ExecutionResult::Success { .. } => true,
        }
    }

    pub fn is_failure(&self) -> bool {
        match self {
            ExecutionResult::Failure { .. } => true,
            ExecutionResult::Success { .. } => false,
        }
    }

    pub fn has_precondition_failure(&self) -> bool {
        match self {
            ExecutionResult::Failure {
                cost,
                execution_effect,
                ..
            } => cost.value() == 0.into() && *execution_effect == Default::default(),
            ExecutionResult::Success { .. } => false,
        }
    }

    pub fn cost(&self) -> Gas {
        match self {
            ExecutionResult::Failure { cost, .. } => *cost,
            ExecutionResult::Success { cost, .. } => *cost,
        }
    }

    pub fn effect(&self) -> &ExecutionEffect {
        match self {
            ExecutionResult::Failure {
                execution_effect, ..
            } => execution_effect,
            ExecutionResult::Success {
                execution_effect, ..
            } => execution_effect,
        }
    }

    pub fn transfers(&self) -> &Vec<TransferAddr> {
        match self {
            ExecutionResult::Failure { transfers, .. } => transfers,
            ExecutionResult::Success { transfers, .. } => transfers,
        }
    }

    pub fn with_cost(self, cost: Gas) -> Self {
        match self {
            ExecutionResult::Failure {
                error,
                execution_effect,
                transfers,
                ..
            } => ExecutionResult::Failure {
                error,
                execution_effect,
                transfers,
                cost,
            },
            ExecutionResult::Success {
                execution_effect,
                transfers,
                ..
            } => ExecutionResult::Success {
                execution_effect,
                transfers,
                cost,
            },
        }
    }

    pub fn with_effect(self, execution_effect: ExecutionEffect) -> Self {
        match self {
            ExecutionResult::Failure {
                error,
                cost,
                transfers,
                ..
            } => ExecutionResult::Failure {
                error,
                execution_effect,
                transfers,
                cost,
            },
            ExecutionResult::Success {
                cost, transfers, ..
            } => ExecutionResult::Success {
                execution_effect,
                transfers,
                cost,
            },
        }
    }

    pub fn with_transfers(self, transfers: Vec<TransferAddr>) -> Self {
        match self {
            ExecutionResult::Failure {
                error,
                execution_effect,
                cost,
                ..
            } => ExecutionResult::Failure {
                error,
                execution_effect,
                transfers,
                cost,
            },
            ExecutionResult::Success {
                cost,
                execution_effect,
                ..
            } => ExecutionResult::Success {
                execution_effect,
                transfers,
                cost,
            },
        }
    }

    pub fn as_error(&self) -> Option<&error::Error> {
        match self {
            ExecutionResult::Failure { error, .. } => Some(error),
            ExecutionResult::Success { .. } => None,
        }
    }

    /// Consumes [`ExecutionResult`] instance and optionally returns [`error::Error`] instance for
    /// [`ExecutionResult::Failure`] variant.
    pub fn take_error(self) -> Option<error::Error> {
        match self {
            ExecutionResult::Failure { error, .. } => Some(error),
            ExecutionResult::Success { .. } => None,
        }
    }

    pub fn check_forced_transfer(
        &self,
        payment_purse_balance: Motes,
        gas_price: u64,
    ) -> Option<ForcedTransferResult> {
        let payment_result_cost = match Motes::from_gas(self.cost(), gas_price) {
            Some(cost) => cost,
            None => return Some(ForcedTransferResult::GasConversionOverflow),
        };
        // payment_code_spec_3_b_ii: if (balance of handle payment pay purse) < (gas spent during
        // payment code execution) * gas_price, no session
        let insufficient_balance_to_continue = payment_purse_balance < payment_result_cost;

        match self {
            ExecutionResult::Success { .. } if insufficient_balance_to_continue => {
                // payment_code_spec_4: insufficient payment
                Some(ForcedTransferResult::InsufficientPayment)
            }
            ExecutionResult::Success { .. } => {
                // payment_code_spec_3_b_ii: continue execution
                None
            }
            ExecutionResult::Failure { .. } => {
                // payment_code_spec_3_a: report payment error in the deploy response
                Some(ForcedTransferResult::PaymentFailure)
            }
        }
    }

    pub fn new_payment_code_error(
        error: error::Error,
        max_payment_cost: Motes,
        account_main_purse_balance: Motes,
        gas_cost: Gas,
        account_main_purse_balance_key: Key,
        proposer_main_purse_balance_key: Key,
    ) -> Result<ExecutionResult, CLValueError> {
        let execution_effect = make_payment_error_effects(
            max_payment_cost,
            account_main_purse_balance,
            account_main_purse_balance_key,
            proposer_main_purse_balance_key,
        )?;
        let transfers = Vec::default();
        Ok(ExecutionResult::Failure {
            error,
            execution_effect,
            transfers,
            cost: gas_cost,
        })
    }

    pub fn take_with_ret<T: FromBytes + CLTyped>(self, ret: T) -> (Option<T>, Self) {
        (Some(ret), self)
    }

    pub fn take_without_ret<T: FromBytes + CLTyped>(self) -> (Option<T>, Self) {
        (None, self)
    }
}

impl From<&ExecutionResult> for casper_types::ExecutionResult {
    fn from(ee_execution_result: &ExecutionResult) -> Self {
        match ee_execution_result {
            ExecutionResult::Success {
                execution_effect,
                transfers,
                cost,
            } => casper_types::ExecutionResult::Success {
                effect: execution_effect.into(),
                transfers: transfers.clone(),
                cost: cost.value(),
            },
            ExecutionResult::Failure {
                error,
                execution_effect,
                transfers,
                cost,
            } => casper_types::ExecutionResult::Failure {
                effect: execution_effect.into(),
                transfers: transfers.clone(),
                cost: cost.value(),
                error_message: error.to_string(),
            },
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ExecutionResultBuilderError {
    MissingPaymentExecutionResult,
    MissingSessionExecutionResult,
    MissingFinalizeExecutionResult,
}

pub struct ExecutionResultBuilder {
    payment_execution_result: Option<ExecutionResult>,
    session_execution_result: Option<ExecutionResult>,
    finalize_execution_result: Option<ExecutionResult>,
}

impl Default for ExecutionResultBuilder {
    fn default() -> Self {
        ExecutionResultBuilder {
            payment_execution_result: None,
            session_execution_result: None,
            finalize_execution_result: None,
        }
    }
}

impl ExecutionResultBuilder {
    pub fn new() -> ExecutionResultBuilder {
        ExecutionResultBuilder::default()
    }

    pub fn set_payment_execution_result(&mut self, payment_result: ExecutionResult) -> &mut Self {
        self.payment_execution_result = Some(payment_result);
        self
    }

    pub fn set_session_execution_result(
        &mut self,
        session_execution_result: ExecutionResult,
    ) -> &mut ExecutionResultBuilder {
        self.session_execution_result = Some(session_execution_result);
        self
    }

    pub fn set_finalize_execution_result(
        &mut self,
        finalize_execution_result: ExecutionResult,
    ) -> &mut ExecutionResultBuilder {
        self.finalize_execution_result = Some(finalize_execution_result);
        self
    }

    pub fn total_cost(&self) -> Gas {
        let payment_cost = self
            .payment_execution_result
            .as_ref()
            .map(ExecutionResult::cost)
            .unwrap_or_default();
        let session_cost = self
            .session_execution_result
            .as_ref()
            .map(ExecutionResult::cost)
            .unwrap_or_default();
        payment_cost + session_cost
    }

    pub fn transfers(&self) -> Vec<TransferAddr> {
        self.session_execution_result
            .as_ref()
            .map(ExecutionResult::transfers)
            .cloned()
            .unwrap_or_default()
    }

    pub fn build<R: StateReader<Key, StoredValue>>(
        self,
        reader: &R,
        correlation_id: CorrelationId,
    ) -> Result<ExecutionResult, ExecutionResultBuilderError> {
        let transfers = self.transfers();
        let cost = self.total_cost();
        let mut ops = AdditiveMap::new();
        let mut transforms = AdditiveMap::new();

        let mut ret: ExecutionResult = ExecutionResult::Success {
            execution_effect: Default::default(),
            transfers,
            cost,
        };

        match self.payment_execution_result {
            Some(result) => {
                if result.is_failure() {
                    return Ok(result);
                } else {
                    Self::add_effects(&mut ops, &mut transforms, result.effect());
                }
            }
            None => return Err(ExecutionResultBuilderError::MissingPaymentExecutionResult),
        };

        // session_code_spec_3: only include session exec effects if there is no session
        // exec error
        match self.session_execution_result {
            Some(result) => {
                if result.is_failure() {
                    ret = result.with_cost(cost);
                } else {
                    Self::add_effects(&mut ops, &mut transforms, result.effect());
                }
            }
            None => return Err(ExecutionResultBuilderError::MissingSessionExecutionResult),
        };

        match self.finalize_execution_result {
            Some(result) => {
                if result.is_failure() {
                    // payment_code_spec_5_a: Finalization Error should only ever be raised here
                    return Ok(ExecutionResult::precondition_failure(
                        error::Error::Finalization,
                    ));
                } else {
                    Self::add_effects(&mut ops, &mut transforms, result.effect());
                }
            }
            None => return Err(ExecutionResultBuilderError::MissingFinalizeExecutionResult),
        }

        // Remove redundant writes to allow more opportunity to commute
        let reduced_effect = Self::reduce_identity_writes(ops, transforms, reader, correlation_id);

        Ok(ret.with_effect(reduced_effect))
    }

    fn add_effects(
        ops: &mut AdditiveMap<Key, Op>,
        transforms: &mut AdditiveMap<Key, Transform>,
        effect: &ExecutionEffect,
    ) {
        for (k, op) in effect.ops.iter() {
            ops.insert_add(*k, *op);
        }
        for (k, t) in effect.transforms.iter() {
            transforms.insert_add(*k, t.clone())
        }
    }

    /// In the case we are writing the same value as was there originally,
    /// it is equivalent to having a `Transform::Identity` and `Op::Read`.
    /// This function makes that reduction before returning the `ExecutionEffect`.
    fn reduce_identity_writes<R: StateReader<Key, StoredValue>>(
        mut ops: AdditiveMap<Key, Op>,
        mut transforms: AdditiveMap<Key, Transform>,
        reader: &R,
        correlation_id: CorrelationId,
    ) -> ExecutionEffect {
        let kvs: Vec<(Key, StoredValue)> = transforms
            .keys()
            .filter_map(|k| match transforms.get(k) {
                Some(Transform::Write(_)) => reader
                    .read(correlation_id, k)
                    .ok()
                    .and_then(|maybe_v| maybe_v.map(|v| (*k, v))),
                _ => None,
            })
            .collect();

        for (k, old_value) in kvs {
            if let Some(Transform::Write(new_value)) = transforms.remove(&k) {
                if new_value == old_value {
                    transforms.insert(k, Transform::Identity);
                    ops.insert(k, Op::Read);
                } else {
                    transforms.insert(k, Transform::Write(new_value));
                }
            }
        }

        ExecutionEffect::new(ops, transforms)
    }
}
