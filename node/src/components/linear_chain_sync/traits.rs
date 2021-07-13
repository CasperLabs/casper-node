use crate::{
    effect::{
        announcements::{ContractRuntimeAnnouncement, ControlAnnouncement},
        requests::{
            BlockValidationRequest, ContractRuntimeRequest, FetcherRequest, StateStoreRequest,
            StorageRequest,
        },
    },
    types::{Block, BlockByHeight},
};

pub trait ReactorEventT<I>:
    From<StorageRequest>
    + From<FetcherRequest<I, Block>>
    + From<FetcherRequest<I, BlockByHeight>>
    + From<BlockValidationRequest<I>>
    + From<ContractRuntimeRequest>
    + From<StateStoreRequest>
    + From<ControlAnnouncement>
    + From<ContractRuntimeAnnouncement>
    + Send
{
}

impl<I, REv> ReactorEventT<I> for REv where
    REv: From<StorageRequest>
        + From<FetcherRequest<I, Block>>
        + From<FetcherRequest<I, BlockByHeight>>
        + From<BlockValidationRequest<I>>
        + From<ContractRuntimeRequest>
        + From<StateStoreRequest>
        + From<ControlAnnouncement>
        + From<ContractRuntimeAnnouncement>
        + Send
{
}
