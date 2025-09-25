use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_channel::{SendError, Sender};
use kaspa_addresses::Address;
use kaspa_consensus_core::{
    constants::{SOMPI_PER_KASPA, STORAGE_MASS_PARAMETER, TX_VERSION},
    sign::sign,
    subnets::SUBNETWORK_ID_NATIVE,
    tx::{MutableTransaction, Transaction, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry},
};
use kaspa_core::{info, warn};
use kaspa_grpc_client::{ClientPool, GrpcClient};
use kaspa_notify::subscription::context::SubscriptionContext;
use kaspa_rpc_core::{api::rpc::RpcApi, notify::mode::NotificationMode};
use kaspa_txscript::pay_to_address_script;
use parking_lot::Mutex;
use rayon::prelude::*;
use tokio::task::yield_now;
use tokio::time::{interval, Instant, MissedTickBehavior};

use crate::config::Config;
use crate::metrics::MetricsCollector;

pub struct BenchmarkEngine {
    config: Arc<Config>,
    rpc_client: Arc<GrpcClient>,
    client_pool: ClientPool<TxSubmission>,
    metrics: Arc<MetricsCollector>,
    pending_txs: Arc<Mutex<HashMap<TransactionOutpoint, Instant>>>,
}

struct TxSubmission {
    tx: Transaction,
    inputs: Vec<TransactionOutpoint>,
    input_entries: Vec<UtxoEntry>,
    metrics: Arc<MetricsCollector>,
    pending_map: Arc<Mutex<HashMap<TransactionOutpoint, Instant>>>,
}

struct UtxoPlan {
    required: usize,
    achievable: usize,
    utxo_size: u64,
}

impl BenchmarkEngine {
    pub async fn new(config: Arc<Config>) -> Result<Self, Box<dyn std::error::Error>> {
        let subscription_context = SubscriptionContext::new();

        // Create main RPC client
        let rpc_client = Arc::new(
            GrpcClient::connect_with_args(
                NotificationMode::Direct,
                config.rpc_server.clone(),
                Some(subscription_context.clone()),
                true,
                None,
                false,
                Some(500_000),
                Default::default(),
            )
            .await?,
        );

        info!("Connected to RPC server at {}", config.rpc_server);

        // Get network info
        let info = rpc_client.get_block_dag_info().await?;
        info!("Network: {}, DAA Score: {}", info.network, info.virtual_daa_score);

        // Create client pool for submissions
        let mut rpc_clients = Vec::with_capacity(config.client_pool_size);
        for _ in 0..config.client_pool_size {
            rpc_clients.push(Arc::new(
                GrpcClient::connect_with_args(
                    NotificationMode::Direct,
                    config.rpc_server.clone(),
                    Some(subscription_context.clone()),
                    true,
                    None,
                    false,
                    Some(500_000),
                    Default::default(),
                )
                .await?,
            ));
        }

        let client_pool = ClientPool::new(rpc_clients, config.channel_capacity);

        Ok(Self {
            config,
            rpc_client,
            client_pool,
            metrics: Arc::new(MetricsCollector::new()),
            pending_txs: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    async fn run_split_phase(
        &self,
        current_utxos: &mut Vec<(TransactionOutpoint, UtxoEntry)>,
        desired_count: usize,
        output_value: u64,
        min_outputs_per_tx: usize,
        phase_label: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if output_value == 0 || min_outputs_per_tx == 0 {
            return Ok(());
        }

        const MAX_PARALLEL_UTXOS: usize = 128;
        const MAX_PARALLEL_TXS: usize = 64;

        let max_outputs_per_tx = self.max_outputs_for_value(output_value);
        let effective_min_outputs = if max_outputs_per_tx < min_outputs_per_tx { 1 } else { min_outputs_per_tx };
        if max_outputs_per_tx < effective_min_outputs {
            warn!(
                "[{phase_label}] Mass limits prevent creating {} outputs of {:.2} KAS each",
                effective_min_outputs,
                output_value as f64 / SOMPI_PER_KASPA as f64
            );
            return Ok(());
        }

        let mut iteration = 0usize;
        let mut no_selection_backoffs = 0usize;
        let mut last_count_for_backoff = current_utxos.len();

        while current_utxos.len() < desired_count {
            iteration += 1;
            current_utxos.sort_by(|a, b| b.1.amount.cmp(&a.1.amount));

            let remaining_needed = desired_count.saturating_sub(current_utxos.len());
            let mut selections: Vec<(Vec<(TransactionOutpoint, UtxoEntry)>, usize)> = Vec::new();
            let mut total_outputs_planned = 0usize;

            {
                let mut pending = self.pending_txs.lock();
                for (idx, (outpoint, entry)) in current_utxos.iter().enumerate() {
                    if idx >= MAX_PARALLEL_UTXOS || selections.len() >= MAX_PARALLEL_TXS || total_outputs_planned >= remaining_needed {
                        break;
                    }

                    if pending.contains_key(outpoint) {
                        continue;
                    }

                    let available = entry.amount;

                    // First, try single-input split if it can create at least the minimum outputs
                    let mut outputs_to_create =
                        max_outputs_per_tx.min(remaining_needed.saturating_sub(total_outputs_planned)).max(effective_min_outputs);

                    while outputs_to_create >= effective_min_outputs {
                        let fee = self.calculate_fee(1, outputs_to_create);
                        let required = output_value * outputs_to_create as u64 + fee;
                        if available >= required {
                            break;
                        }
                        outputs_to_create -= 1;
                    }

                    if outputs_to_create >= effective_min_outputs {
                        pending.insert(*outpoint, Instant::now());
                        selections.push((vec![(*outpoint, entry.clone())], outputs_to_create));
                        total_outputs_planned += outputs_to_create;
                    }
                }
            }

            // If we still need more outputs or had no single-input candidates, try multi-input grouping of small UTXOs
            if selections.is_empty() || total_outputs_planned < remaining_needed {
                let mut pending = self.pending_txs.lock();
                // Collect small candidates not yet pending
                let mut small: Vec<(TransactionOutpoint, UtxoEntry)> = current_utxos
                    .iter()
                    .filter(|(op, _)| !pending.contains_key(op))
                    .map(|(op, e)| (*op, e.clone()))
                    .collect();
                // Sort ascending by amount to group small ones
                small.sort_by(|a, b| a.1.amount.cmp(&b.1.amount));

                let mut idx = 0usize;
                while idx < small.len() && selections.len() < MAX_PARALLEL_TXS && total_outputs_planned < remaining_needed {
                    // Form a group of 2-4 inputs
                    let mut group: Vec<(TransactionOutpoint, UtxoEntry)> = Vec::new();
                    let mut sum = 0u64;
                    let start_idx = idx;
                    while idx < small.len() && group.len() < 4 {
                        let (op, e) = {
                            let (op_ref, e_ref) = &small[idx];
                            (*op_ref, e_ref.clone())
                        };
                        if pending.contains_key(&op) {
                            idx += 1;
                            continue;
                        }
                        group.push((op, e.clone()));
                        sum = sum.saturating_add(e.amount);
                        idx += 1;
                        if group.len() >= 2 {
                            // Try to see if we can produce at least effective_min_outputs
                            let mut outputs_to_create =
                                max_outputs_per_tx.min(remaining_needed.saturating_sub(total_outputs_planned)).max(effective_min_outputs);
                            while outputs_to_create >= effective_min_outputs {
                                let fee = self.calculate_fee(group.len(), outputs_to_create);
                                let required = output_value * outputs_to_create as u64 + fee;
                                if sum >= required {
                                    break;
                                }
                                outputs_to_create -= 1;
                            }
                            if outputs_to_create >= effective_min_outputs {
                                for (op_used, _) in &group {
                                    pending.insert(*op_used, Instant::now());
                                }
                                selections.push((group.clone(), outputs_to_create));
                                total_outputs_planned += outputs_to_create;
                                break;
                            }
                        }
                    }
                    if start_idx == idx {
                        idx += 1;
                    }
                }
            }

            if selections.is_empty() {
                warn!(
                    "[{phase_label}] No eligible UTXOs available (have: {}, need: {}). Waiting for confirmations...",
                    current_utxos.len(),
                    desired_count
                );

                if no_selection_backoffs < 200 {
                    no_selection_backoffs += 1;
                    let wait_ms = self.config.block_interval_ms.max(200);
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    *current_utxos = self.fetch_spendable_utxos_with_min_conf(1).await?;
                    self.metrics.update_utxo_count(current_utxos.len());
                    if current_utxos.len() > last_count_for_backoff {
                        no_selection_backoffs = 0;
                        last_count_for_backoff = current_utxos.len();
                    }
                    continue;
                } else {
                    warn!(
                        "[{phase_label}] Giving up after waiting for confirmations without new eligible UTXOs"
                    );
                    break;
                }
            }

            info!("[{phase_label}] iteration {} planning {} txs for ~{} outputs", iteration, selections.len(), total_outputs_planned);

            let addr = self.config.address.clone();
            let keypair = self.config.keypair;
            let prepared: Vec<(Transaction, Vec<TransactionOutpoint>, Vec<UtxoEntry>, usize)> = selections
                .into_par_iter()
                .map(|(inputs_group, outputs_to_create)| {
                    // Build split tx
                    let inputs: Vec<TransactionInput> = inputs_group
                        .iter()
                        .map(|(outpoint, _)| TransactionInput { previous_outpoint: *outpoint, signature_script: vec![], sequence: 0, sig_op_count: 1 })
                        .collect();

                    let mut outputs = Vec::with_capacity(outputs_to_create);
                    for _ in 0..outputs_to_create {
                        outputs.push(TransactionOutput { value: output_value, script_public_key: pay_to_address_script(&addr) });
                    }

                    let fee = self.calculate_fee(inputs.len(), outputs_to_create);
                    let required = output_value * outputs_to_create as u64 + fee;
                    let sum_inputs: u64 = inputs_group.iter().map(|(_, e)| e.amount).sum();
                    let change = sum_inputs.saturating_sub(required);
                    if change > 0 && !outputs.is_empty() {
                        // Fold change into the last output to keep storage mass low
                        let last = outputs.last_mut().unwrap();
                        last.value = last.value.saturating_add(change);
                    }

                    let unsigned_tx = Transaction::new(TX_VERSION, inputs, outputs, 0, SUBNETWORK_ID_NATIVE, 0, vec![]);
                    let utxo_entries: Vec<UtxoEntry> = inputs_group.iter().map(|(_, e)| e.clone()).collect();
                    let signed_tx = sign(MutableTransaction::with_entries(unsigned_tx, utxo_entries.clone()), keypair);

                    let input_points: Vec<TransactionOutpoint> = inputs_group.iter().map(|(op, _)| *op).collect();
                    (signed_tx.tx, input_points, utxo_entries, outputs_to_create)
                })
                .collect();

            let mut submission_futures = Vec::with_capacity(prepared.len());
            for (tx, _, _, _) in &prepared {
                let client = self.rpc_client.clone();
                let tx_clone = tx.clone();
                submission_futures.push(async move { client.submit_transaction(tx_clone.as_ref().into(), false).await });
            }

            let results = futures::future::join_all(submission_futures).await;
            let mut failed_inputs: Vec<(TransactionOutpoint, UtxoEntry)> = Vec::new();
            let mut successful_outputs = 0usize;
            let mut rejected_over_mass = 0usize;
            let mut other_rejected = 0usize;

            for ((_, inputs_used, entries_used, outputs_expected), result) in prepared.into_iter().zip(results.into_iter()) {
                match result {
                    Ok(id) => {
                        successful_outputs += outputs_expected;
                        info!("[{phase_label}] Accepted split tx ({outputs_expected} outputs): {}", id);
                    }
                    Err(e) => {
                        let es = e.to_string();
                        if es.contains("storage mass") && es.contains("max allowed") {
                            rejected_over_mass += 1;
                        } else {
                            other_rejected += 1;
                        }
                        for (inp, ent) in inputs_used.into_iter().zip(entries_used.into_iter()) {
                            failed_inputs.push((inp, ent));
                        }
                    }
                }
            }

            if !failed_inputs.is_empty() {
                let mut pending = self.pending_txs.lock();
                for (input, entry) in failed_inputs {
                    pending.remove(&input);
                    if !current_utxos.iter().any(|(out, _)| out == &input) {
                        current_utxos.push((input, entry));
                    }
                }
            }

            if rejected_over_mass > 0 || other_rejected > 0 {
                info!(
                    "[{phase_label}] Submission results: accepted_outputs={}, rejected_over_mass={}, other_rejected={}",
                    successful_outputs, rejected_over_mass, other_rejected
                );
            }

            if successful_outputs == 0 {
                warn!("[{phase_label}] No transactions accepted in iteration {}; backing off", iteration);
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }

            info!("[{phase_label}] Submitting wave yielded {} accepted outputs", successful_outputs);

            let split_conf = self.config.confirmation_depth.min(1u64);
            let mut wait_ms = (self.config.utxo_refresh_interval.as_millis() as u64)
                .max(self.config.block_interval_ms.saturating_mul(2))
                .max(1000u64);
            if split_conf > 1 {
                wait_ms = wait_ms.max(self.config.block_interval_ms.saturating_mul(split_conf));
            }
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;

            *current_utxos = self.fetch_spendable_utxos_with_min_conf(split_conf).await?;
            self.metrics.update_utxo_count(current_utxos.len());
            if current_utxos.len() > last_count_for_backoff {
                no_selection_backoffs = 0;
                last_count_for_backoff = current_utxos.len();
            }
            info!(
                "[{phase_label}] Progress: {} / {} UTXOs ({:.1}%)",
                current_utxos.len(),
                desired_count,
                (current_utxos.len() as f64 / desired_count as f64) * 100.0
            );

            if iteration > desired_count * 6 {
                warn!("[{phase_label}] Reached iteration limit while preparing UTXOs");
                break;
            }
        }

        Ok(())
    }

    pub async fn prepare_utxos(&self, target_count: usize, utxo_size_hint: Option<u64>) -> Result<(), Box<dyn std::error::Error>> {
        info!("Preparing {} UTXOs for benchmarking", target_count);

        let mut current_utxos = self.fetch_spendable_utxos().await?;
        info!("Current UTXO count: {}", current_utxos.len());

        if current_utxos.len() >= target_count {
            info!("Already have {} UTXOs, no splitting needed", current_utxos.len());
            return Ok(());
        }

        let total_balance: u64 = current_utxos.iter().map(|(_, entry)| entry.amount).sum();
        info!("Total balance: {:.2} KAS", total_balance as f64 / SOMPI_PER_KASPA as f64);

        let estimated_fees = (target_count as u64) * 100_000;
        let available_for_utxos = total_balance.saturating_sub(estimated_fees);

        let min_utxo_size = 20_000_000;
        let ideal_utxo_size = 145_000_000;
        let max_utxo_size = ideal_utxo_size * 2;
        let calculated_size = available_for_utxos / target_count as u64;

        let mut utxo_size = if let Some(hint) = utxo_size_hint {
            hint
        } else if target_count > 500 && calculated_size > 50_000_000 {
            50_000_000
        } else if calculated_size >= ideal_utxo_size {
            ideal_utxo_size
        } else if calculated_size >= min_utxo_size {
            calculated_size
        } else {
            warn!(
                "Insufficient balance for {} UTXOs. Each would be {:.4} KAS",
                target_count,
                calculated_size as f64 / SOMPI_PER_KASPA as f64
            );
            min_utxo_size
        };

        utxo_size = utxo_size.clamp(min_utxo_size, max_utxo_size);

        info!("Using UTXO size: {:.2} KAS", utxo_size as f64 / SOMPI_PER_KASPA as f64);

        let coarse_value = (utxo_size.saturating_mul(3)).clamp(utxo_size + min_utxo_size / 2, max_utxo_size);
        let coarse_applicable = coarse_value > utxo_size && current_utxos.iter().any(|(_, entry)| entry.amount >= coarse_value * 2);

        if coarse_applicable {
            let coarse_goal = target_count.max(current_utxos.len()) + target_count;
            info!(
                "Coarse split phase: aiming for ~{} UTXOs with {:.2} KAS outputs",
                coarse_goal,
                coarse_value as f64 / SOMPI_PER_KASPA as f64
            );
            self.run_split_phase(&mut current_utxos, coarse_goal, coarse_value, 2, "coarse").await?;
        }

        if current_utxos.len() < target_count {
            info!(
                "Fine split phase: targeting {} UTXOs with {:.2} KAS outputs",
                target_count,
                utxo_size as f64 / SOMPI_PER_KASPA as f64
            );
            self.run_split_phase(&mut current_utxos, target_count, utxo_size, 2, "fine").await?;
        }

        // Skip tail cleanup with single-output splits: they don't increase UTXO count

        info!("\n=== UTXO Preparation Complete ===");
        info!("Final UTXO count: {}", current_utxos.len());
        info!("Target was: {}", target_count);

        if current_utxos.len() < target_count {
            warn!("Could not reach target. You may need more initial balance.");
            let shortfall = target_count - current_utxos.len();
            let additional_kas_needed = (shortfall as f64 * utxo_size as f64) / SOMPI_PER_KASPA as f64;
            info!("Estimated additional KAS needed: {:.2}", additional_kas_needed);
        }

        Ok(())
    }

    pub async fn run(&self, duration_seconds: u64) -> Result<(), Box<dyn std::error::Error>> {
        self.ensure_utxo_inventory(duration_seconds).await?;

        // Start metrics timing and TPS calculator
        self.metrics.mark_start();
        let metrics = self.metrics.clone();
        tokio::spawn(async move {
            metrics.start_tps_calculator().await;
        });

        let tx_sender = self.start_submission_pool();

        // Fetch initial UTXOs
        let mut utxos = self.fetch_spendable_utxos().await?;
        info!("Starting with {} UTXOs", utxos.len());

        if utxos.is_empty() {
            warn!("No UTXOs available! Please fund the address: {}", self.config.address);
            return Ok(());
        }

        let start_time = Instant::now();
        let mut ticker = interval(Duration::from_millis(self.config.tick_ms));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut last_refresh = Instant::now();
        let mut next_utxo_index = 0;
        let mut rate_accumulator = 0.0f64;

        loop {
            ticker.tick().await;

            if duration_seconds > 0 && start_time.elapsed().as_secs() >= duration_seconds {
                info!("Benchmark duration completed");
                break;
            }

            if last_refresh.elapsed() >= self.config.utxo_refresh_interval {
                utxos = self.fetch_spendable_utxos().await?;
                last_refresh = Instant::now();
                next_utxo_index = 0;
                info!("Refreshed UTXOs: {}", utxos.len());
            }

            rate_accumulator += (self.config.target_tps as f64) * (self.config.tick_ms as f64) / 1000.0;
            let mut tx_budget = rate_accumulator.floor() as usize;
            rate_accumulator -= tx_budget as f64;

            if tx_budget == 0 {
                continue;
            }

            if utxos.len().saturating_sub(next_utxo_index) < tx_budget {
                utxos = self.fetch_spendable_utxos().await?;
                last_refresh = Instant::now();
                next_utxo_index = 0;
            }

            let available = utxos.len().saturating_sub(next_utxo_index);
            if available == 0 {
                warn!("Out of spendable UTXOs; waiting for confirmations");
                continue;
            }
            if available < tx_budget {
                tx_budget = available;
            }

            let queued = self.generate_and_send_txs(tx_budget, &tx_sender, &mut utxos, &mut next_utxo_index).await;

            self.metrics.update_utxo_count(utxos.len().saturating_sub(next_utxo_index));

            if queued == 0 {
                yield_now().await;
            }
        }

        Ok(())
    }

    async fn ensure_utxo_inventory(&self, _duration_seconds: u64) -> Result<(), Box<dyn std::error::Error>> {
        let utxos = self.fetch_spendable_utxos().await?;
        let current_count = utxos.len();
        let mut amounts: Vec<u64> = utxos.iter().map(|(_, entry)| entry.amount).collect();
        amounts.sort_unstable();
        let total_balance: u64 = amounts.iter().copied().sum();
        let min_amount = amounts.first().copied().unwrap_or(0);
        let lower_quartile_amount = if amounts.is_empty() {
            0
        } else {
            let idx = amounts.len() / 4;
            amounts[idx]
        };

        if current_count == 0 {
            warn!("No confirmed UTXOs detected; ensure the address is funded before running the benchmark");
        }

        let plan = self.derive_utxo_plan(current_count, total_balance, min_amount, lower_quartile_amount);

        if current_count >= plan.required {
            info!("UTXO inventory ready: {} available (>= required {})", current_count, plan.required);
            return Ok(());
        }

        if plan.required > plan.achievable {
            warn!(
                "Target TPS {} requests ~{} UTXOs but available balance (~{:.2} KAS) supports only {}. Throughput may be limited.",
                self.config.target_tps,
                plan.required,
                total_balance as f64 / SOMPI_PER_KASPA as f64,
                plan.achievable
            );
        }

        let target = plan.required.min(plan.achievable.max(current_count));
        if target <= current_count {
            info!("UTXO inventory constrained to {} due to balance limits", current_count);
            return Ok(());
        }

        info!(
            "Auto-prep: have {} confirmed UTXOs, targeting {} (~{:.2} KAS each)",
            current_count,
            target,
            plan.utxo_size as f64 / SOMPI_PER_KASPA as f64
        );

        self.prepare_utxos(target, Some(plan.utxo_size)).await?;
        Ok(())
    }

    fn derive_utxo_plan(&self, current_count: usize, total_balance: u64, min_amount: u64, lower_quartile_amount: u64) -> UtxoPlan {
        let required_min = self.compute_required_utxo_count();
        let mut utxo_size = (total_balance / required_min.max(1) as u64).max(1);

        let fee_two_outputs = self.calculate_fee(1, 2);

        let min_utxo_size = 20_000_000;
        let max_utxo_size = 220_000_000;

        if utxo_size < min_utxo_size && lower_quartile_amount > 0 {
            utxo_size = lower_quartile_amount.max(min_utxo_size);
        } else if utxo_size > max_utxo_size {
            utxo_size = max_utxo_size;
        }

        if min_amount > 0 {
            let cap = ((min_amount.saturating_sub(fee_two_outputs)) / 2).max(min_utxo_size);
            utxo_size = utxo_size.min(cap);
        }

        utxo_size = utxo_size.clamp(min_utxo_size, max_utxo_size);

        let achievable = (total_balance / utxo_size).max(current_count as u64) as usize;

        UtxoPlan { required: required_min.max(current_count), achievable, utxo_size }
    }

    fn compute_required_utxo_count(&self) -> usize {
        let tps = self.config.target_tps.max(1);
        let confirmations = self.config.confirmation_depth.max(1);
        let block_interval = (self.config.block_interval_ms as f64 / 1000.0).clamp(0.05, 2.0);

        let concurrency = (tps as f64) * block_interval * confirmations as f64;
        let buffer = (tps as f64 * 4.0).max(concurrency * 6.0);
        buffer.ceil() as usize
    }

    async fn generate_and_send_txs(
        &self,
        count: usize,
        tx_sender: &Sender<TxSubmission>,
        utxos: &mut Vec<(TransactionOutpoint, UtxoEntry)>,
        next_index: &mut usize,
    ) -> usize {
        if count == 0 {
            return 0;
        }

        let mut selected = Vec::new();
        {
            let mut pending = self.pending_txs.lock();
            while selected.len() < count {
                if *next_index >= utxos.len() {
                    break;
                }

                let (outpoint, entry) = &utxos[*next_index];
                *next_index += 1;

                if pending.contains_key(outpoint) {
                    continue;
                }

                pending.insert(*outpoint, Instant::now());
                selected.push((*outpoint, entry.clone()));
            }
        }

        if selected.is_empty() {
            return 0;
        }

        let metrics_arc = self.metrics.clone();
        let pending_arc = self.pending_txs.clone();

        let submissions: Vec<TxSubmission> = selected
            .into_par_iter()
            .map(|(outpoint, entry)| {
                let tx = self.create_spam_transaction(outpoint, entry.clone());
                TxSubmission {
                    tx,
                    inputs: vec![outpoint],
                    input_entries: vec![entry],
                    metrics: metrics_arc.clone(),
                    pending_map: pending_arc.clone(),
                }
            })
            .collect();

        let mut queued = 0usize;
        let mut requeue = Vec::new();
        let mut iter = submissions.into_iter();

        while let Some(submission) = iter.next() {
            match tx_sender.send(submission).await {
                Ok(_) => {
                    queued += 1;
                }
                Err(SendError(failed_submission)) => {
                    let TxSubmission { inputs, input_entries, pending_map, .. } = failed_submission;
                    Self::release_inputs(&pending_map, inputs, input_entries, &mut requeue);

                    for leftover in iter {
                        let TxSubmission { inputs, input_entries, pending_map, .. } = leftover;
                        Self::release_inputs(&pending_map, inputs, input_entries, &mut requeue);
                    }

                    warn!("Submission channel closed; re-queuing {} inputs", requeue.len());
                    break;
                }
            }
        }

        if !requeue.is_empty() {
            let insert_pos = *next_index;
            utxos.splice(insert_pos..insert_pos, requeue.into_iter());
        }

        self.metrics.record_enqueued(queued);
        queued
    }

    fn create_spam_transaction(&self, outpoint: TransactionOutpoint, utxo: UtxoEntry) -> Transaction {
        let fee = self.calculate_fee(1, 1);
        let output_amount = utxo.amount.saturating_sub(fee);

        let inputs = vec![TransactionInput { previous_outpoint: outpoint, signature_script: vec![], sequence: 0, sig_op_count: 1 }];

        let outputs = vec![TransactionOutput { value: output_amount, script_public_key: pay_to_address_script(&self.config.address) }];

        let unsigned_tx = Transaction::new(TX_VERSION, inputs, outputs, 0, SUBNETWORK_ID_NATIVE, 0, vec![]);

        let signed_tx = sign(MutableTransaction::with_entries(unsigned_tx, vec![utxo]), self.config.keypair);

        signed_tx.tx
    }

    fn release_inputs(
        pending_map: &Arc<Mutex<HashMap<TransactionOutpoint, Instant>>>,
        inputs: Vec<TransactionOutpoint>,
        entries: Vec<UtxoEntry>,
        requeue: &mut Vec<(TransactionOutpoint, UtxoEntry)>,
    ) {
        let mut pending = pending_map.lock();
        for (input, entry) in inputs.into_iter().zip(entries.into_iter()) {
            pending.remove(&input);
            requeue.push((input, entry));
        }
    }

    fn calculate_fee(&self, num_inputs: usize, num_outputs: usize) -> u64 {
        let mass = 200 + 1_000 * num_inputs as u64 + 50 * num_outputs as u64;
        mass * 10
    }

    fn max_outputs_for_value(&self, value: u64) -> usize {
        const MAX_TX_MASS: u64 = 100_000;
        if value == 0 {
            return 0;
        }

        // Conservative mass-per-output using ceil division
        let mass_per_output = (STORAGE_MASS_PARAMETER.saturating_add(value.max(1) - 1)) / value.max(1);
        if mass_per_output == 0 {
            return 10;
        }

        // Use headroom to account for base tx mass and serialization overhead
        let effective_max = (MAX_TX_MASS as f64 * 0.85) as u64;
        let mut limit = effective_max / mass_per_output;
        if limit == 0 {
            limit = 1;
        }

        limit = limit.min(10);
        limit.max(1) as usize
    }

    async fn fetch_spendable_utxos(&self) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, Box<dyn std::error::Error>> {
        self.fetch_spendable_utxos_with_min_conf(self.config.confirmation_depth).await
    }

    async fn fetch_spendable_utxos_with_min_conf(
        &self,
        min_confirmations: u64,
    ) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, Box<dyn std::error::Error>> {
        let resp = self.rpc_client.get_utxos_by_addresses(vec![self.config.address.clone()]).await?;
        let dag_info = self.rpc_client.get_block_dag_info().await?;
        let available: HashSet<TransactionOutpoint> =
            resp.iter().map(|entry| TransactionOutpoint::from(entry.outpoint)).collect();

        let mut pending = self.pending_txs.lock();
        let ttl = self.config.pending_ttl;
        pending.retain(|outpoint, inserted| if available.contains(outpoint) { inserted.elapsed() <= ttl } else { false });

        let mut utxos = Vec::with_capacity(available.len());
        for entry in resp {
            let outpoint = TransactionOutpoint::from(entry.outpoint);
            if pending.contains_key(&outpoint) {
                continue;
            }

            let confirmations = dag_info.virtual_daa_score.saturating_sub(entry.utxo_entry.block_daa_score);
            if confirmations < min_confirmations {
                continue;
            }

            utxos.push((outpoint, UtxoEntry::from(entry.utxo_entry)));
        }

        Ok(utxos)
    }

    fn start_submission_pool(&self) -> Sender<TxSubmission> {
        let _handles = self.client_pool.start(|client, submission: TxSubmission| async move {
            let TxSubmission { tx, inputs, metrics, pending_map, .. } = submission;
            match client.submit_transaction(tx.as_ref().into(), false).await {
                Ok(_) => {
                    metrics.record_accepted();
                }
                Err(e) => {
                    if e.to_string().contains("already in mempool") {
                        metrics.record_accepted();
                    } else {
                        metrics.record_failed();
                        {
                            let mut pending = pending_map.lock();
                            for input in &inputs {
                                pending.remove(input);
                            }
                        }
                        warn!("Failed to submit transaction: {}", e);
                    }
                }
            }
            false
        });

        self.client_pool.sender()
    }

    pub fn metrics_collector(&self) -> Arc<MetricsCollector> {
        self.metrics.clone()
    }

    pub async fn sweep_all_utxos(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Sweeping all UTXOs to consolidate");

        // Fetch all UTXOs
        let utxos = self.fetch_spendable_utxos().await?;

        if utxos.is_empty() {
            warn!("No UTXOs to sweep");
            return Ok(());
        }

        info!("Found {} UTXOs to sweep", utxos.len());

        let total_amount: u64 = utxos.iter().map(|(_, entry)| entry.amount).sum();
        info!("Total amount: {} KAS", total_amount as f64 / SOMPI_PER_KASPA as f64);

        // Process in batches to avoid mass limits
        const MAX_INPUTS_PER_TX: usize = 84; // Safe limit to stay under mass
        let mut transactions = Vec::new();

        for chunk in utxos.chunks(MAX_INPUTS_PER_TX) {
            let inputs: Vec<_> = chunk
                .iter()
                .map(|(outpoint, _)| TransactionInput {
                    previous_outpoint: *outpoint,
                    signature_script: vec![],
                    sequence: 0,
                    sig_op_count: 1,
                })
                .collect();

            let chunk_amount: u64 = chunk.iter().map(|(_, entry)| entry.amount).sum();
            let fee = self.calculate_fee(inputs.len(), 1);
            let output_amount = chunk_amount.saturating_sub(fee);

            let outputs =
                vec![TransactionOutput { value: output_amount, script_public_key: pay_to_address_script(&self.config.address) }];

            let unsigned_tx = Transaction::new(TX_VERSION, inputs, outputs, 0, SUBNETWORK_ID_NATIVE, 0, vec![]);

            let utxo_entries: Vec<_> = chunk.iter().map(|(_, entry)| entry.clone()).collect();
            let signed_tx = sign(MutableTransaction::with_entries(unsigned_tx, utxo_entries), self.config.keypair);

            transactions.push(signed_tx.tx);
        }

        info!("Submitting {} sweep transactions", transactions.len());

        // Submit all transactions
        for (i, tx) in transactions.iter().enumerate() {
            match self.rpc_client.submit_transaction(tx.as_ref().into(), false).await {
                Ok(id) => info!("Sweep transaction {} submitted: {}", i + 1, id),
                Err(e) => warn!("Failed to submit sweep transaction {}: {}", i + 1, e),
            }
        }

        // Wait a bit for confirmations
        info!("Waiting 10 seconds for sweep to complete...");
        tokio::time::sleep(Duration::from_secs(10)).await;

        // Check final UTXO count
        let final_utxos = self.fetch_spendable_utxos().await?;
        info!("Sweep complete. Final UTXO count: {}", final_utxos.len());

        Ok(())
    }

    pub async fn send_funds(&self, address: String, amount: f64) -> Result<(), Box<dyn std::error::Error>> {
        info!("Sending {} KAS to {}", amount, address);

        // Parse destination address
        let dest_address = Address::try_from(address.as_str()).map_err(|e| format!("Invalid address: {}", e))?;

        // Convert amount to sompi
        let amount_sompi = (amount * SOMPI_PER_KASPA as f64) as u64;

        // Fetch UTXOs
        let utxos = self.fetch_spendable_utxos().await?;
        if utxos.is_empty() {
            return Err("No UTXOs available".into());
        }

        // Select UTXOs to cover the amount
        let mut selected_utxos = Vec::new();
        let mut selected_amount = 0u64;

        for (outpoint, entry) in utxos.iter() {
            selected_utxos.push((*outpoint, entry.clone()));
            selected_amount += entry.amount;

            // Estimate fee and check if we have enough
            let fee = self.calculate_fee(selected_utxos.len(), 2); // 2 outputs: destination + change
            if selected_amount >= amount_sompi + fee {
                break;
            }
        }

        let fee = self.calculate_fee(selected_utxos.len(), 2);

        if selected_amount < amount_sompi + fee {
            return Err(format!(
                "Insufficient funds. Need {} KAS (including fee), have {} KAS",
                (amount_sompi + fee) as f64 / SOMPI_PER_KASPA as f64,
                selected_amount as f64 / SOMPI_PER_KASPA as f64
            )
            .into());
        }

        // Build transaction
        let inputs: Vec<_> = selected_utxos
            .iter()
            .map(|(outpoint, _)| TransactionInput {
                previous_outpoint: *outpoint,
                signature_script: vec![],
                sequence: 0,
                sig_op_count: 1,
            })
            .collect();

        let mut outputs = vec![TransactionOutput { value: amount_sompi, script_public_key: pay_to_address_script(&dest_address) }];

        // Add change output if necessary
        let change = selected_amount - amount_sompi - fee;
        if change > 100_000 {
            // Min change threshold
            outputs.push(TransactionOutput { value: change, script_public_key: pay_to_address_script(&self.config.address) });
        }

        let unsigned_tx = Transaction::new(TX_VERSION, inputs, outputs, 0, SUBNETWORK_ID_NATIVE, 0, vec![]);

        let utxo_entries: Vec<_> = selected_utxos.iter().map(|(_, entry)| entry.clone()).collect();
        let signed_tx = sign(MutableTransaction::with_entries(unsigned_tx, utxo_entries), self.config.keypair);

        // Submit transaction
        match self.rpc_client.submit_transaction(signed_tx.tx.as_ref().into(), false).await {
            Ok(id) => {
                info!("Transaction submitted: {}", id);
                info!("Sent {} KAS to {}", amount, address);
                if change > 100_000 {
                    info!("Change: {} KAS", change as f64 / SOMPI_PER_KASPA as f64);
                }
                info!("Fee: {} KAS", fee as f64 / SOMPI_PER_KASPA as f64);
            }
            Err(e) => return Err(format!("Failed to submit transaction: {}", e).into()),
        }

        Ok(())
    }
}
