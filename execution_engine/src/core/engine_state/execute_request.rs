use std::mem;

use casper_types::{ProtocolVersion, PublicKey, SecretKey};

use super::deploy_item::DeployItem;
use crate::shared::newtypes::Blake2bHash;

#[derive(Debug, Clone)]
pub struct ExecuteRequest {
    pub parent_state_hash: Blake2bHash,
    pub block_time: u64,
    pub deploys: Vec<DeployItem>,
    pub protocol_version: ProtocolVersion,
    pub proposer: PublicKey,
}

impl ExecuteRequest {
    pub fn new(
        parent_state_hash: Blake2bHash,
        block_time: u64,
        deploys: Vec<DeployItem>,
        protocol_version: ProtocolVersion,
        proposer: PublicKey,
    ) -> Self {
        Self {
            parent_state_hash,
            block_time,
            deploys,
            protocol_version,
            proposer,
        }
    }

    pub fn take_deploys(&mut self) -> Vec<DeployItem> {
        mem::take(&mut self.deploys)
    }

    pub fn deploys(&self) -> &Vec<DeployItem> {
        &self.deploys
    }
}

impl Default for ExecuteRequest {
    fn default() -> Self {
        let proposer_secret_key =
            SecretKey::ed25519_from_bytes([0; SecretKey::ED25519_LENGTH]).unwrap();
        let proposer = PublicKey::from(&proposer_secret_key);
        Self {
            parent_state_hash: Blake2bHash::new(&[]),
            block_time: 0,
            deploys: vec![],
            protocol_version: Default::default(),
            proposer,
        }
    }
}
