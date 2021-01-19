use datasize::DataSize;
use prometheus::{self, Histogram, HistogramOpts, IntGauge, Registry};
use tracing::{debug, warn};

use super::Reactor;

///Metrics for memory usage for the joiner
#[derive(Debug)]
pub struct MemoryMetrics {
    /// Total estimated heap memory usage.
    mem_total: IntGauge,

    /// Estimated heap memory usage of metrics component.
    mem_metrics: IntGauge,
    /// Estimated heap memory usage of network component.
    mem_network: IntGauge,
    /// Estimated heap memory usage of small_network component.
    mem_small_network: IntGauge,
    /// Estimated heap memory usage of address_gossiper component.
    mem_address_gossiper: IntGauge,
    /// Estimated heap memory usage of the configuration for the validator node.
    mem_config: IntGauge,
    /// Estimated heap memory usage for the chainspec loader component.
    mem_chainspec_loader: IntGauge,
    /// Estimated heap memory usage of storage component.
    mem_storage: IntGauge,
    /// Estimated heap memory usage of the contract runtime component.
    mem_contract_runtime: IntGauge,
    /// Estimated heap memory usage of the linear chain fetcher component.
    mem_linear_chain_fetcher: IntGauge,
    /// Estimated heap memory usage of linear chain sync.
    mem_linear_chain_sync: IntGauge,
    /// Estimated heap memory usage of block validator component.
    mem_block_validator: IntGauge,
    /// Estimated heap memory usage of deploy fetcher component.
    mem_deploy_fetcher: IntGauge,
    /// Estimated heap memory usage of block executor component.
    mem_block_executor: IntGauge,
    /// Estimated heap memory usage of linear chain component.
    mem_linear_chain: IntGauge,
    /// Estimated heap memory usage of consensus component.
    mem_consensus: IntGauge,

    /// Histogram detailing how long it took to estimate memory usage.
    mem_estimator_runtime_s: Histogram,

    /// Instance of registry component to unregister from when being dropped.
    registry: Registry,
}

impl MemoryMetrics {
    /// Initialize a new set of memory metrics for the joiner.
    pub(super) fn new(registry: Registry) -> Result<Self, prometheus::Error> {
        let mem_total = IntGauge::new("joiner_mem_total", "total memory usage in bytes")?;
        let mem_metrics = IntGauge::new("joiner_mem_metrics", "metrics memory usage in bytes")?;
        let mem_network = IntGauge::new("joiner_mem_network", "network memory usage in bytes")?;
        let mem_small_network = IntGauge::new(
            "joiner_mem_small_network",
            "small network memory usage in bytes",
        )?;
        let mem_address_gossiper = IntGauge::new(
            "joiner_mem_address_gossiper",
            "address_gossiper memory usage in bytes",
        )?;
        let mem_config = IntGauge::new("joiner_mem_config", "config memory usage in bytes")?;
        let mem_chainspec_loader = IntGauge::new(
            "joiner_mem_chainspec_loader",
            "chainspec_loader memory usage in bytes",
        )?;
        let mem_storage = IntGauge::new("joiner_mem_storage", "storage memory usage in bytes")?;
        let mem_contract_runtime = IntGauge::new(
            "joiner_mem_contract_runtime",
            "contract_runtime memory usage in bytes",
        )?;
        let mem_linear_chain_fetcher = IntGauge::new(
            "joiner_mem_linear_chain_fetcher",
            "linear_chain_fetcher memory usage in bytes",
        )?;
        let mem_linear_chain_sync = IntGauge::new(
            "joiner_mem_linear_chain_sync",
            "linear_chain_sync memory usage in bytes",
        )?;
        let mem_block_validator = IntGauge::new(
            "joiner_mem_block_validator",
            "block_validator memory usage in bytes",
        )?;
        let mem_deploy_fetcher = IntGauge::new(
            "joiner_mem_deploy_fetcher",
            "deploy_fetcher memory usage in bytes",
        )?;
        let mem_block_executor =
            IntGauge::new("mem_block_executor", "block_executor memory usage in bytes")?;
        let mem_linear_chain = IntGauge::new(
            "joiner_mem_linear_chain",
            "linear_chain memory usage in bytes",
        )?;
        let mem_consensus =
            IntGauge::new("joiner_mem_consensus", "memory usage of consensus in bytes")?;
        let mem_estimator_runtime_s = Histogram::with_opts(
            HistogramOpts::new(
                "joiner_mem_estimator_runtime_s",
                "time taken to estimate memory usage, in seconds",
            )
            // Create buckets from four nano second to eight seconds
            .buckets(prometheus::exponential_buckets(0.000_000_004, 2.0, 32)?),
        )?;

        registry.register(Box::new(mem_total.clone()))?;
        registry.register(Box::new(mem_metrics.clone()))?;
        registry.register(Box::new(mem_network.clone()))?;
        registry.register(Box::new(mem_small_network.clone()))?;
        registry.register(Box::new(mem_address_gossiper.clone()))?;
        registry.register(Box::new(mem_config.clone()))?;
        registry.register(Box::new(mem_chainspec_loader.clone()))?;
        registry.register(Box::new(mem_storage.clone()))?;
        registry.register(Box::new(mem_contract_runtime.clone()))?;
        registry.register(Box::new(mem_linear_chain_fetcher.clone()))?;
        registry.register(Box::new(mem_linear_chain_sync.clone()))?;
        registry.register(Box::new(mem_block_validator.clone()))?;
        registry.register(Box::new(mem_deploy_fetcher.clone()))?;
        registry.register(Box::new(mem_block_executor.clone()))?;
        registry.register(Box::new(mem_linear_chain.clone()))?;
        registry.register(Box::new(mem_consensus.clone()))?;

        Ok(MemoryMetrics {
            mem_total,
            mem_metrics,
            mem_network,
            mem_small_network,
            mem_address_gossiper,
            mem_config,
            mem_chainspec_loader,
            mem_storage,
            mem_contract_runtime,
            mem_linear_chain_fetcher,
            mem_linear_chain_sync,
            mem_block_validator,
            mem_deploy_fetcher,
            mem_block_executor,
            mem_linear_chain,
            mem_consensus,
            mem_estimator_runtime_s,
            registry,
        })
    }

    /// Estimates the memory usage and updates metrics.
    pub(super) fn estimate(&self, reactor: &Reactor) {
        let timer = self.mem_estimator_runtime_s.start_timer();

        let metrics = reactor.metrics.estimate_heap_size() as i64;
        let network = reactor.network.estimate_heap_size() as i64;
        let small_network = reactor.small_network.estimate_heap_size() as i64;
        let address_gossiper = reactor.address_gossiper.estimate_heap_size() as i64;
        let config = reactor.config.estimate_heap_size() as i64;
        let chainspec_loader = reactor.chainspec_loader.estimate_heap_size() as i64;
        let storage = reactor.storage.estimate_heap_size() as i64;
        let contract_runtime = reactor.contract_runtime.estimate_heap_size() as i64;
        let linear_chain_fetcher = reactor.linear_chain_fetcher.estimate_heap_size() as i64;
        let linear_chain_sync = reactor.linear_chain_sync.estimate_heap_size() as i64;
        let block_validator = reactor.block_validator.estimate_heap_size() as i64;
        let deploy_fetcher = reactor.deploy_fetcher.estimate_heap_size() as i64;
        let block_executor = reactor.block_executor.estimate_heap_size() as i64;
        let linear_chain = reactor.linear_chain.estimate_heap_size() as i64;
        let consensus = reactor.consensus.estimate_heap_size() as i64;

        let total = metrics
            + network
            + small_network
            + address_gossiper
            + config
            + chainspec_loader
            + storage
            + contract_runtime
            + linear_chain_fetcher
            + linear_chain_sync
            + block_validator
            + deploy_fetcher
            + block_executor
            + linear_chain
            + consensus;

        self.mem_total.set(total);
        self.mem_metrics.set(metrics);
        self.mem_network.set(network);
        self.mem_small_network.set(small_network);
        self.mem_address_gossiper.set(address_gossiper);
        self.mem_config.set(config);
        self.mem_chainspec_loader.set(chainspec_loader);
        self.mem_storage.set(storage);
        self.mem_contract_runtime.set(contract_runtime);
        self.mem_linear_chain_fetcher.set(linear_chain_fetcher);
        self.mem_linear_chain_sync.set(linear_chain_sync);
        self.mem_block_validator.set(block_validator);
        self.mem_deploy_fetcher.set(deploy_fetcher);
        self.mem_block_executor.set(block_executor);
        self.mem_linear_chain.set(linear_chain);
        self.mem_consensus.set(consensus);

        // Stop the timer explicitly, don't count logging.
        let duration_s = timer.stop_and_record();

        debug!(
        %total,
        %duration_s,
        %metrics,
        %network,
        %small_network,
        %address_gossiper,
        %config ,
        %chainspec_loader,
        %storage ,
        %contract_runtime,
        %linear_chain_fetcher,
        %linear_chain_sync,
        %block_validator,
        %deploy_fetcher,
        %block_executor,
        %linear_chain,
        %consensus,
        "Collected new set of memory metrics for the joiner");
    }
}

impl Drop for MemoryMetrics {
    fn drop(&mut self) {
        self.registry
            .unregister(Box::new(self.mem_total.clone()))
            .unwrap_or_else(|err| warn!(%err, "did not expect mem_total to fail"));
        self.registry
            .unregister(Box::new(self.mem_metrics.clone()))
            .unwrap_or_else(
                |err| warn!(%err, "did not expect deregistering joiner_mem_metrics to fail"),
            );
        self.registry
            .unregister(Box::new(self.mem_network.clone()))
            .unwrap_or_else(
                |err| warn!(%err, "did not expect deregistering joiner_mem_network to fail"),
            );
        self.registry
            .unregister(Box::new(self.mem_small_network.clone()))
            .unwrap_or_else(
                |err| warn!(%err, "did not expect deregistering joiner_mem_small_network to fail"),
            );
        self.registry
            .unregister(Box::new(self.mem_address_gossiper.clone()))
            .unwrap_or_else(|err| warn!(%err, "did not expect deregistering joiner_mem_address_gossiper to fail"));
        self.registry
            .unregister(Box::new(self.mem_config.clone()))
            .unwrap_or_else(
                |err| warn!(%err, "did not expect deregistering joiner_mem_config to fail"),
            );
        self.registry
            .unregister(Box::new(self.mem_chainspec_loader.clone()))
            .unwrap_or_else(|err| warn!(%err, "did not expect deregistering joiner_mem_chainspec_loader to fail"));
        self.registry
            .unregister(Box::new(self.mem_storage.clone()))
            .unwrap_or_else(
                |err| warn!(%err, "did not expect deregistering joiner_mem_storage to fail"),
            );
        self.registry
            .unregister(Box::new(self.mem_contract_runtime.clone()))
            .unwrap_or_else(|err| warn!(%err, "did not expect deregistering joiner_mem_contract_runtime to fail"));
        self.registry
            .unregister(Box::new(self.mem_linear_chain_fetcher.clone()))
            .unwrap_or_else(|err| warn!(%err, "did not expect deregistering joiner_mem_linear_chain_fetcher to fail"));
        self.registry
            .unregister(Box::new(self.mem_linear_chain_sync.clone()))
            .unwrap_or_else(|err| warn!(%err, "did not expect deregistering joiner_mem_linear_chain_sync to fail"));
        self.registry
            .unregister(Box::new(self.mem_block_validator.clone()))
            .unwrap_or_else(|err| warn!(%err, "did not expect deregistering joiner_mem_block_validator to fail"));
        self.registry
            .unregister(Box::new(self.mem_deploy_fetcher.clone()))
            .unwrap_or_else(
                |err| warn!(%err, "did not expect deregistering joiner_mem_deploy_fetcher to fail"),
            );
        self.registry
            .unregister(Box::new(self.mem_block_executor.clone()))
            .unwrap_or_else(
                |err| warn!(%err, "did not expect deregistering joiner_mem_block_executor to fail"),
            );
        self.registry
            .unregister(Box::new(self.mem_linear_chain.clone()))
            .unwrap_or_else(
                |err| warn!(%err, "did not expect deregistering joiner_mem_linear_chain to fail"),
            );
        self.registry
            .unregister(Box::new(self.mem_consensus.clone()))
            .unwrap_or_else(
                |err| warn!(%err, "did not expect deregistering joiner_mem_consensus to fail"),
            );
    }
}
