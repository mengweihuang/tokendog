//! Kubernetes service discovery for dynamic worker management.
//!
//! Watches K8s pods via the `kube` runtime watcher, maintains a tracked set
//! of `PodInfo` (identity = name + uid), and adds/removes workers from shared
//! `Arc<RwLock<Vec<Arc<Worker>>>>` pools. A periodic reconciliation loop
//! catches events missed by the watcher.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::Duration,
};

use futures::{StreamExt, TryStreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::{Api, ListParams},
    runtime::{
        watcher::{watcher, Config},
        WatchStreamExt,
    },
    Client,
};
use tokio::{task, time};
use tracing::{debug, error, info, warn};

use crate::worker::Worker;

// ── ServiceDiscoveryConfig ───────────────────────────────────────────────────

/// Configuration for Kubernetes service discovery.
#[derive(Debug, Clone)]
pub struct ServiceDiscoveryConfig {
    /// Whether service discovery is enabled.
    pub enabled: bool,
    /// Label selector for worker pods in regular mode: `key=value` pairs.
    pub selector: HashMap<String, String>,
    /// Interval between periodic reconciliation cycles.
    pub check_interval: Duration,
    /// Port workers listen on inside pods.
    pub port: u16,
    /// K8s namespace to watch (None = all namespaces).
    pub namespace: Option<String>,
    /// Whether Prefill-Decode separation mode is active.
    pub pd_mode: bool,
    /// Label selector for prefill pods (PD mode).
    pub prefill_selector: HashMap<String, String>,
    /// Label selector for decode pods (PD mode).
    pub decode_selector: HashMap<String, String>,
}

impl ServiceDiscoveryConfig {
    /// Build a label selector string for K8s list calls.
    ///
    /// In regular mode, uses the worker selector directly.
    /// In PD mode, uses labels common to both prefill and decode selectors
    /// so a single list call covers both pod types.
    fn list_label_selector(&self) -> String {
        if self.pd_mode {
            self.prefill_selector
                .iter()
                .filter(|(k, v)| self.decode_selector.get(*k) == Some(*v))
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(",")
        } else {
            self.selector
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(",")
        }
    }
}

impl Default for ServiceDiscoveryConfig {
    fn default() -> Self {
        ServiceDiscoveryConfig {
            enabled: false,
            selector: HashMap::new(),
            check_interval: Duration::from_secs(60),
            port: 8000,
            namespace: None,
            pd_mode: false,
            prefill_selector: HashMap::new(),
            decode_selector: HashMap::new(),
        }
    }
}

// ── PodType ──────────────────────────────────────────────────────────────────

/// Classification of a worker pod in PD mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PodType {
    Prefill,
    Decode,
    Regular,
}

// ── PodInfo ──────────────────────────────────────────────────────────────────

/// Information about a discovered Kubernetes pod.
///
/// Identity is defined by `(name, uid)` — the uid changes on every pod restart,
/// so StatefulSet pods that keep the same name across restarts are detected as
/// different entities by the reconciliation system.
#[derive(Debug, Clone)]
pub struct PodInfo {
    pub name: String,
    pub uid: String,
    pub ip: String,
    pub status: String,
    pub is_ready: bool,
    pub pod_type: Option<PodType>,
}

impl PartialEq for PodInfo {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.uid == other.uid
    }
}

impl Eq for PodInfo {}

impl std::hash::Hash for PodInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.uid.hash(state);
    }
}

impl PodInfo {
    /// Check whether a pod's labels match all entries in the given selector.
    fn matches_selector(pod: &Pod, selector: &HashMap<String, String>) -> bool {
        if selector.is_empty() {
            return false;
        }
        pod.metadata
            .labels
            .as_ref()
            .is_some_and(|labels| selector.iter().all(|(k, v)| labels.get(k) == Some(v)))
    }

    /// Determine whether this pod should be included based on the discovery config.
    pub fn should_include(pod: &Pod, config: &ServiceDiscoveryConfig) -> bool {
        if config.pd_mode {
            if config.prefill_selector.is_empty() && config.decode_selector.is_empty() {
                warn!("PD mode enabled but both prefill_selector and decode_selector are empty");
                return false;
            }
            Self::matches_selector(pod, &config.prefill_selector)
                || Self::matches_selector(pod, &config.decode_selector)
        } else {
            if config.selector.is_empty() {
                warn!("Regular mode enabled but selector is empty");
                return false;
            }
            Self::matches_selector(pod, &config.selector)
        }
    }

    /// Build a `PodInfo` from a Kubernetes `Pod` object.
    pub fn from_pod(pod: &Pod, config: Option<&ServiceDiscoveryConfig>) -> Option<Self> {
        let name = pod.metadata.name.clone()?;
        let uid = match pod.metadata.uid.clone() {
            Some(uid) => uid,
            None => {
                warn!(
                    "Pod {} has no UID, skipping -- cannot track identity for reconciliation",
                    name
                );
                return None;
            }
        };
        let status = pod.status.clone()?;
        let pod_ip = status.pod_ip?;

        let is_ready = if let Some(conditions) = &status.conditions {
            conditions
                .iter()
                .any(|condition| condition.type_ == "Ready" && condition.status == "True")
        } else {
            false
        };

        let pod_status = status.phase.unwrap_or_else(|| "Unknown".to_string());

        let pod_type = config.map(|cfg| {
            if cfg.pd_mode {
                if Self::matches_selector(pod, &cfg.prefill_selector) {
                    PodType::Prefill
                } else if Self::matches_selector(pod, &cfg.decode_selector) {
                    PodType::Decode
                } else {
                    PodType::Regular
                }
            } else {
                PodType::Regular
            }
        });

        Some(PodInfo {
            name,
            uid,
            ip: pod_ip,
            status: pod_status,
            is_ready,
            pod_type,
        })
    }

    /// Returns `true` if this pod is ready and running.
    pub fn is_healthy(&self) -> bool {
        self.is_ready && self.status == "Running"
    }

    /// Build the worker URL from this pod's IP and the provided port.
    pub fn worker_url(&self, port: u16) -> String {
        format!("{}:{}", self.ip, port)
    }
}

// ── Watcher config helper ────────────────────────────────────────────────────

/// Build a kube watcher `Config` that pushes the given label selector down
/// to the API server. An empty selector falls back to `Config::default()`
/// (no server-side label filtering).
fn build_watcher_config(watcher_kind: &str, label_selector: &str) -> Config {
    info!(
        "Starting K8s {} watcher | selector: '{}'",
        watcher_kind, label_selector
    );
    if label_selector.is_empty() {
        Config::default()
    } else {
        Config::default().labels(label_selector)
    }
}

// ── Shared pool helpers ──────────────────────────────────────────────────────

/// Shared worker pool type used by service discovery and AppState.
pub type SharedWorkerPool = Arc<RwLock<Vec<Arc<Worker>>>>;

/// Add a worker URL to the shared pool if not already present.
fn add_worker_to_pool(pool: &SharedWorkerPool, url: &str) -> bool {
    let mut workers = match pool.write() {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to acquire worker pool lock for add: {}", e);
            return false;
        }
    };
    // Avoid duplicates
    if workers.iter().any(|w| w.url == url) {
        return false;
    }
    workers.push(Arc::new(Worker::new(url.to_string())));
    info!("Worker added to pool | url: {} | total: {}", url, workers.len());
    true
}

/// Remove a worker URL from the shared pool.
fn remove_worker_from_pool(pool: &SharedWorkerPool, url: &str) -> bool {
    let mut workers = match pool.write() {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to acquire worker pool lock for remove: {}", e);
            return false;
        }
    };
    let prev_len = workers.len();
    workers.retain(|w| w.url != url);
    let removed = workers.len() < prev_len;
    if removed {
        info!(
            "Worker removed from pool | url: {} | remaining: {}",
            url,
            workers.len()
        );
    }
    removed
}

/// Select the correct pool for a pod based on its type and PD mode.
fn select_pool<'a>(
    pod_info: &PodInfo,
    pd_mode: bool,
    regular_pool: &'a SharedWorkerPool,
    prefill_pool: Option<&'a SharedWorkerPool>,
    decode_pool: Option<&'a SharedWorkerPool>,
) -> &'a SharedWorkerPool {
    if pd_mode {
        match pod_info.pod_type {
            Some(PodType::Prefill) => prefill_pool.unwrap_or(regular_pool),
            Some(PodType::Decode) => decode_pool.unwrap_or(regular_pool),
            _ => regular_pool,
        }
    } else {
        regular_pool
    }
}

// ── Event handlers ───────────────────────────────────────────────────────────

/// Handle a pod create/update event: add healthy workers, evict old UIDs on restart.
async fn handle_pod_event(
    pod_info: &PodInfo,
    tracked_pods: Arc<std::sync::Mutex<HashSet<PodInfo>>>,
    regular_pool: SharedWorkerPool,
    prefill_pool: Option<SharedWorkerPool>,
    decode_pool: Option<SharedWorkerPool>,
    port: u16,
    pd_mode: bool,
) {
    let worker_url = pod_info.worker_url(port);
    let target_pool = select_pool(
        pod_info,
        pd_mode,
        &regular_pool,
        prefill_pool.as_ref(),
        decode_pool.as_ref(),
    );

    if pod_info.is_healthy() {
        let (should_add, evicted_url) = {
            let mut tracker = match tracked_pods.lock() {
                Ok(t) => t,
                Err(e) => {
                    error!("Failed to acquire tracked_pods lock: {}", e);
                    return;
                }
            };

            if tracker.contains(pod_info) {
                (false, None)
            } else {
                // Check for same-name pod with different UID (restart).
                let old = tracker
                    .iter()
                    .find(|p| p.name == pod_info.name && p.uid != pod_info.uid)
                    .cloned();
                if let Some(ref old) = old {
                    tracker.remove(old);
                }
                tracker.insert(pod_info.clone());
                (true, old.map(|o| o.worker_url(port)))
            }
        };

        // Evict the old-UID pod from the pool.
        if let Some(ref old_url) = evicted_url {
            info!(
                "Evicting restarted pod {} (old uid) | url: {}",
                pod_info.name, old_url
            );
            remove_worker_from_pool(target_pool, old_url);
        }

        if should_add {
            info!(
                "Adding pod: {} | type: {:?} | url: {}",
                pod_info.name, pod_info.pod_type, worker_url
            );
            add_worker_to_pool(target_pool, &worker_url);
        }
    }
}

/// Handle a pod deletion event: remove the worker from the pool.
async fn handle_pod_deletion(
    pod_info: &PodInfo,
    tracked_pods: Arc<std::sync::Mutex<HashSet<PodInfo>>>,
    regular_pool: SharedWorkerPool,
    prefill_pool: Option<SharedWorkerPool>,
    decode_pool: Option<SharedWorkerPool>,
    port: u16,
    pd_mode: bool,
) {
    let worker_url = pod_info.worker_url(port);
    let target_pool = select_pool(
        pod_info,
        pd_mode,
        &regular_pool,
        prefill_pool.as_ref(),
        decode_pool.as_ref(),
    );

    let was_tracked = {
        let mut tracked = match tracked_pods.lock() {
            Ok(t) => t,
            Err(e) => {
                error!(
                    "Failed to acquire tracked_pods lock during deletion: {}",
                    e
                );
                return;
            }
        };
        tracked.remove(pod_info)
    };

    if was_tracked {
        info!(
            "Removing pod: {} | type: {:?} | url: {}",
            pod_info.name, pod_info.pod_type, worker_url
        );
        remove_worker_from_pool(target_pool, &worker_url);
    }
}

// ── Reconciliation ───────────────────────────────────────────────────────────

/// Build the set of live pods from a K8s pod list, filtering by config selectors
/// and excluding pods with a deletion timestamp.
fn build_live_pod_set(pod_list: &[Pod], config: &ServiceDiscoveryConfig) -> HashSet<PodInfo> {
    let mut live_pods = HashSet::new();
    for pod in pod_list {
        if !PodInfo::should_include(pod, config) {
            continue;
        }
        if pod.metadata.deletion_timestamp.is_some() {
            continue;
        }
        if let Some(info) = PodInfo::from_pod(pod, Some(config)) {
            live_pods.insert(info);
        }
    }
    live_pods
}

/// Compute the reconciliation diff between tracked and live pod sets.
///
/// Returns `(stale, missing)` where:
/// - `stale`: pods in `tracked` but not in `live` (should be removed)
/// - `missing`: pods in `live` but not in `tracked` that are healthy (should be added)
fn compute_reconciliation_diff(
    tracked: &HashSet<PodInfo>,
    live: &HashSet<PodInfo>,
) -> (Vec<PodInfo>, Vec<PodInfo>) {
    let stale: Vec<PodInfo> = tracked.difference(live).cloned().collect();
    let missing: Vec<PodInfo> = live
        .difference(tracked)
        .filter(|p| p.is_healthy())
        .cloned()
        .collect();
    (stale, missing)
}

/// Reconcile the tracked pod set with actual Kubernetes state.
///
/// Performs a full pod list and compares with `tracked_pods`:
/// - Pods in `tracked_pods` but no longer in K8s → remove from pool
/// - Healthy pods in K8s but missing from `tracked_pods` → add to pool
async fn reconcile_pods(
    pods: &Api<Pod>,
    config: Arc<ServiceDiscoveryConfig>,
    tracked_pods: Arc<std::sync::Mutex<HashSet<PodInfo>>>,
    regular_pool: SharedWorkerPool,
    prefill_pool: Option<SharedWorkerPool>,
    decode_pool: Option<SharedWorkerPool>,
    port: u16,
) {
    let label_selector = config.list_label_selector();
    let list_params = if label_selector.is_empty() {
        ListParams::default()
    } else {
        ListParams::default().labels(&label_selector)
    };
    let pod_list = match pods.list(&list_params).await {
        Ok(list) => list,
        Err(e) => {
            error!("Reconciliation: failed to list pods: {}", e);
            return;
        }
    };

    // Build the set of live pods that match our selectors.
    let live_pods = build_live_pod_set(&pod_list.items, &config);

    // Diff: stale = tracked but not live, missing = live-and-healthy but not tracked
    let (stale, missing) = {
        let tracked = match tracked_pods.lock() {
            Ok(t) => t,
            Err(e) => {
                error!("Reconciliation: failed to acquire lock: {}", e);
                return;
            }
        };
        compute_reconciliation_diff(&tracked, &live_pods)
    };

    if stale.is_empty() && missing.is_empty() {
        debug!("Reconciliation: tracked state is consistent with K8s");
        return;
    }

    info!(
        "Reconciliation: removing {} stale, adding {} missing pods",
        stale.len(),
        missing.len()
    );

    // Remove stale workers.
    for pod_info in &stale {
        let worker_url = pod_info.worker_url(port);
        let target_pool = select_pool(
            pod_info,
            config.pd_mode,
            &regular_pool,
            prefill_pool.as_ref(),
            decode_pool.as_ref(),
        );
        info!(
            "Reconciliation: removing stale pod {} (uid={}) | url: {}",
            pod_info.name, pod_info.uid, worker_url
        );
        remove_worker_from_pool(target_pool, &worker_url);

        // Also remove from tracked set.
        if let Ok(mut tracker) = tracked_pods.lock() {
            tracker.remove(pod_info);
        }
    }

    // Add missing workers.
    for pod_info in &missing {
        handle_pod_event(
            pod_info,
            Arc::clone(&tracked_pods),
            Arc::clone(&regular_pool),
            prefill_pool.clone(),
            decode_pool.clone(),
            port,
            config.pd_mode,
        )
        .await;
    }
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Start the Kubernetes service discovery loop.
///
/// Spawns a background task that watches for pod events and periodically
/// reconciles the tracked state against the K8s API. Returns a `JoinHandle`
/// that can be awaited for graceful shutdown.
///
/// # Arguments
///
/// * `config` - Service discovery configuration.
/// * `regular_pool` - Shared worker pool for regular mode (and fallback in PD mode).
/// * `prefill_pool` - Shared worker pool for prefill pods (PD mode only).
/// * `decode_pool` - Shared worker pool for decode pods (PD mode only).
pub async fn start_service_discovery(
    config: ServiceDiscoveryConfig,
    regular_pool: SharedWorkerPool,
    prefill_pool: Option<SharedWorkerPool>,
    decode_pool: Option<SharedWorkerPool>,
) -> Result<task::JoinHandle<()>, kube::Error> {
    // Install rustls crypto provider for TLS connections to K8s API.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let client = Client::try_default().await?;

    // Log the configured selectors.
    if config.pd_mode {
        let prefill_selector = config
            .prefill_selector
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        let decode_selector = config
            .decode_selector
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        info!(
            "Starting K8s service discovery | PD mode | prefill: '{}' | decode: '{}'",
            prefill_selector, decode_selector
        );
    } else {
        let label_selector = config
            .selector
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        info!(
            "Starting K8s service discovery | selector: '{}'",
            label_selector
        );
    }

    let handle = task::spawn(async move {
        let tracked_pods = Arc::new(std::sync::Mutex::new(HashSet::new()));

        let pods: Api<Pod> = if let Some(namespace) = &config.namespace {
            Api::namespaced(client, namespace)
        } else {
            Api::all(client)
        };

        debug!("K8s service discovery initialized");

        let config_arc = Arc::new(config.clone());
        let port = config.port;

        // Spawn periodic reconciliation supervisor.
        {
            let reconcile_pods_api = pods.clone();
            let reconcile_config = Arc::clone(&config_arc);
            let reconcile_tracked = Arc::clone(&tracked_pods);
            let reconcile_regular = Arc::clone(&regular_pool);
            let reconcile_prefill = prefill_pool.clone();
            let reconcile_decode = decode_pool.clone();
            let reconcile_interval = config.check_interval;

            tokio::spawn(async move {
                loop {
                    let api = reconcile_pods_api.clone();
                    let cfg = Arc::clone(&reconcile_config);
                    let trk = Arc::clone(&reconcile_tracked);
                    let reg = Arc::clone(&reconcile_regular);
                    let pre = reconcile_prefill.clone();
                    let dec = reconcile_decode.clone();

                    let handle = tokio::spawn(async move {
                        // Delay the first tick so the watcher has time to populate initial state.
                        let start = time::Instant::now() + reconcile_interval;
                        let mut interval = time::interval_at(start, reconcile_interval);
                        loop {
                            interval.tick().await;
                            reconcile_pods(
                                &api,
                                Arc::clone(&cfg),
                                Arc::clone(&trk),
                                Arc::clone(&reg),
                                pre.clone(),
                                dec.clone(),
                                port,
                            )
                            .await;
                        }
                    });
                    if let Err(e) = handle.await {
                        error!(
                            "Periodic reconciliation task panicked: {} -- restarting after {}s",
                            e,
                            reconcile_interval.as_secs()
                        );
                        time::sleep(reconcile_interval).await;
                    } else {
                        break;
                    }
                }
            });
            info!(
                "Periodic reconciliation enabled | interval: {}s",
                config.check_interval.as_secs()
            );
        }

        let mut retry_delay = Duration::from_secs(1);
        const MAX_RETRY_DELAY: Duration = Duration::from_secs(300);

        loop {
            let watcher_config =
                build_watcher_config("worker", &config_arc.list_label_selector());
            let watcher_stream = watcher(pods.clone(), watcher_config).applied_objects();

            let config_clone = Arc::clone(&config_arc);
            let tracked_pods_clone = Arc::clone(&tracked_pods);

            let filtered_stream = watcher_stream.filter_map(move |obj_res| {
                let config_inner = Arc::clone(&config_clone);
                async move {
                    match obj_res {
                        Ok(pod) => {
                            if PodInfo::should_include(&pod, &config_inner) {
                                Some(Ok(pod))
                            } else {
                                None
                            }
                        }
                        Err(e) => Some(Err(e)),
                    }
                }
            });

            let tracked_pods_clone2 = Arc::clone(&tracked_pods_clone);
            let config_clone2 = Arc::clone(&config_arc);
            let regular_clone = Arc::clone(&regular_pool);
            let prefill_clone = prefill_pool.clone();
            let decode_clone = decode_pool.clone();

            let watcher_ok = filtered_stream
                .try_for_each(move |pod| {
                    let tracked_pods_inner = Arc::clone(&tracked_pods_clone2);
                    let config_inner = Arc::clone(&config_clone2);
                    let regular_inner = Arc::clone(&regular_clone);
                    let prefill_inner = prefill_clone.clone();
                    let decode_inner = decode_clone.clone();

                    async move {
                        let pod_info = PodInfo::from_pod(&pod, Some(&config_inner));
                        if let Some(pod_info) = pod_info {
                            if pod.metadata.deletion_timestamp.is_some() {
                                handle_pod_deletion(
                                    &pod_info,
                                    tracked_pods_inner,
                                    regular_inner,
                                    prefill_inner,
                                    decode_inner,
                                    port,
                                    config_inner.pd_mode,
                                )
                                .await;
                            } else {
                                handle_pod_event(
                                    &pod_info,
                                    tracked_pods_inner,
                                    regular_inner,
                                    prefill_inner,
                                    decode_inner,
                                    port,
                                    config_inner.pd_mode,
                                )
                                .await;
                            }
                        }
                        Ok(())
                    }
                })
                .await;

            match watcher_ok {
                Ok(()) => {
                    retry_delay = Duration::from_secs(1);
                }
                Err(err) => {
                    error!("Error in Kubernetes watcher: {}", err);
                    warn!(
                        "Retrying in {} seconds with exponential backoff",
                        retry_delay.as_secs()
                    );
                    time::sleep(retry_delay).await;
                    retry_delay = std::cmp::min(retry_delay * 2, MAX_RETRY_DELAY);
                }
            }

            warn!(
                "Kubernetes watcher exited, restarting in {} seconds",
                config_arc.check_interval.as_secs()
            );
            time::sleep(config_arc.check_interval).await;
        }
    });

    Ok(handle)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::{
        api::core::v1::{PodCondition, PodSpec, PodStatus},
        apimachinery::pkg::apis::meta::v1::ObjectMeta,
    };

    fn create_test_pod(
        name: &str,
        ip: &str,
        phase: &str,
        ready_status: &str,
        labels: Option<Vec<(&str, &str)>>,
    ) -> Pod {
        let label_map = labels.map(|pairs| {
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<std::collections::BTreeMap<_, _>>()
        });

        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                uid: Some(format!("uid-{name}")),
                labels: label_map,
                ..Default::default()
            },
            spec: Some(PodSpec::default()),
            status: Some(PodStatus {
                pod_ip: Some(ip.to_string()),
                phase: Some(phase.to_string()),
                conditions: Some(vec![PodCondition {
                    type_: "Ready".to_string(),
                    status: ready_status.to_string(),
                    last_probe_time: None,
                    last_transition_time: None,
                    message: None,
                    reason: None,
                }]),
                ..Default::default()
            }),
        }
    }

    fn make_regular_config() -> ServiceDiscoveryConfig {
        let mut selector = HashMap::new();
        selector.insert("app".to_string(), "sglang".to_string());
        ServiceDiscoveryConfig {
            enabled: true,
            selector,
            pd_mode: false,
            ..Default::default()
        }
    }

    fn make_pod_info(name: &str, uid: &str, ip: &str, status: &str, is_ready: bool) -> PodInfo {
        PodInfo {
            name: name.into(),
            uid: uid.into(),
            ip: ip.into(),
            status: status.into(),
            is_ready,
            pod_type: Some(PodType::Regular),
        }
    }

    // ── PodInfo tests ────────────────────────────────────────────────────────

    #[test]
    fn test_pod_info_from_pod_valid() {
        let pod = create_test_pod("test-pod", "10.0.0.1", "Running", "True", None);
        let info = PodInfo::from_pod(&pod, None).unwrap();
        assert_eq!(info.name, "test-pod");
        assert_eq!(info.ip, "10.0.0.1");
        assert_eq!(info.status, "Running");
        assert!(info.is_ready);
    }

    #[test]
    fn test_pod_info_from_pod_missing_name() {
        let pod = create_test_pod("test-pod", "10.0.0.1", "Running", "True", None);
        let mut pod_no_name = pod.clone();
        pod_no_name.metadata.name = None;
        assert!(PodInfo::from_pod(&pod_no_name, None).is_none());
    }

    #[test]
    fn test_pod_info_from_pod_missing_uid() {
        let pod = create_test_pod("test-pod", "10.0.0.1", "Running", "True", None);
        let mut pod_no_uid = pod.clone();
        pod_no_uid.metadata.uid = None;
        assert!(PodInfo::from_pod(&pod_no_uid, None).is_none());
    }

    #[test]
    fn test_pod_info_from_pod_not_ready() {
        let pod = create_test_pod("test-pod", "10.0.0.1", "Running", "False", None);
        let info = PodInfo::from_pod(&pod, None).unwrap();
        assert!(!info.is_ready);
    }

    #[test]
    fn test_pod_info_is_healthy() {
        let healthy = make_pod_info("p1", "uid-1", "1.1.1.1", "Running", true);
        assert!(healthy.is_healthy());

        let not_ready = make_pod_info("p2", "uid-2", "1.1.1.2", "Running", false);
        assert!(!not_ready.is_healthy());

        let not_running = make_pod_info("p3", "uid-3", "1.1.1.3", "Pending", true);
        assert!(!not_running.is_healthy());
    }

    #[test]
    fn test_pod_info_identity_based_equality() {
        let pod1 = make_pod_info("pod-a", "uid-1", "10.0.0.1", "Running", true);
        let pod2 = make_pod_info("pod-a", "uid-1", "10.0.0.2", "Pending", false);
        let pod3 = make_pod_info("pod-b", "uid-2", "10.0.0.1", "Running", true);

        // Same (name, uid) → equal regardless of mutable fields
        assert_eq!(pod1, pod2);
        // Different (name, uid) → not equal
        assert_ne!(pod1, pod3);
    }

    #[test]
    fn test_pod_info_should_include_regular_mode() {
        let config = make_regular_config();
        let matching = create_test_pod(
            "pod-1", "10.0.0.1", "Running", "True",
            Some(vec![("app", "sglang")]),
        );
        let non_matching = create_test_pod(
            "pod-2", "10.0.0.2", "Running", "True",
            Some(vec![("app", "other")]),
        );

        assert!(PodInfo::should_include(&matching, &config));
        assert!(!PodInfo::should_include(&non_matching, &config));
    }

    #[test]
    fn test_pod_info_should_include_pd_mode() {
        let mut prefill = HashMap::new();
        prefill.insert("app".to_string(), "sglang".to_string());
        prefill.insert("component".to_string(), "prefill".to_string());
        let mut decode = HashMap::new();
        decode.insert("app".to_string(), "sglang".to_string());
        decode.insert("component".to_string(), "decode".to_string());

        let config = ServiceDiscoveryConfig {
            enabled: true,
            pd_mode: true,
            prefill_selector: prefill,
            decode_selector: decode,
            ..Default::default()
        };

        let prefill_pod = create_test_pod(
            "prefill-0", "10.0.0.1", "Running", "True",
            Some(vec![("app", "sglang"), ("component", "prefill")]),
        );
        let decode_pod = create_test_pod(
            "decode-0", "10.0.0.2", "Running", "True",
            Some(vec![("app", "sglang"), ("component", "decode")]),
        );
        let other_pod = create_test_pod(
            "other-0", "10.0.0.3", "Running", "True",
            Some(vec![("app", "other")]),
        );

        assert!(PodInfo::should_include(&prefill_pod, &config));
        assert!(PodInfo::should_include(&decode_pod, &config));
        assert!(!PodInfo::should_include(&other_pod, &config));
    }

    // ── PodInfo PD type classification ───────────────────────────────────────

    #[test]
    fn test_pod_info_from_pod_pd_prefill() {
        let mut prefill = HashMap::new();
        prefill.insert("app".to_string(), "sglang".to_string());
        prefill.insert("component".to_string(), "prefill".to_string());
        let mut decode = HashMap::new();
        decode.insert("app".to_string(), "sglang".to_string());

        let config = ServiceDiscoveryConfig {
            pd_mode: true,
            prefill_selector: prefill,
            decode_selector: decode,
            ..Default::default()
        };

        let pod = create_test_pod(
            "prefill-0", "10.0.0.1", "Running", "True",
            Some(vec![("app", "sglang"), ("component", "prefill")]),
        );
        let info = PodInfo::from_pod(&pod, Some(&config)).unwrap();
        assert_eq!(info.pod_type, Some(PodType::Prefill));
    }

    #[test]
    fn test_pod_info_from_pod_pd_decode() {
        let mut prefill = HashMap::new();
        prefill.insert("app".to_string(), "sglang".to_string());
        let mut decode = HashMap::new();
        decode.insert("app".to_string(), "sglang".to_string());
        decode.insert("component".to_string(), "decode".to_string());

        let config = ServiceDiscoveryConfig {
            pd_mode: true,
            prefill_selector: prefill,
            decode_selector: decode,
            ..Default::default()
        };

        let pod = create_test_pod(
            "decode-0", "10.0.0.1", "Running", "True",
            Some(vec![("app", "sglang"), ("component", "decode")]),
        );
        let info = PodInfo::from_pod(&pod, Some(&config)).unwrap();
        assert_eq!(info.pod_type, Some(PodType::Decode));
    }

    // ── Config tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_service_discovery_config_default() {
        let config = ServiceDiscoveryConfig::default();
        assert!(!config.enabled);
        assert!(config.selector.is_empty());
        assert_eq!(config.check_interval, Duration::from_secs(60));
        assert_eq!(config.port, 8000);
        assert!(config.namespace.is_none());
        assert!(!config.pd_mode);
    }

    #[test]
    fn test_list_label_selector_regular_mode() {
        let config = make_regular_config();
        assert_eq!(config.list_label_selector(), "app=sglang");
    }

    #[test]
    fn test_list_label_selector_pd_mode_common_labels() {
        let mut prefill = HashMap::new();
        prefill.insert("app".to_string(), "sglang".to_string());
        prefill.insert("component".to_string(), "prefill".to_string());
        let mut decode = HashMap::new();
        decode.insert("app".to_string(), "sglang".to_string());
        decode.insert("component".to_string(), "decode".to_string());
        let config = ServiceDiscoveryConfig {
            pd_mode: true,
            prefill_selector: prefill,
            decode_selector: decode,
            ..Default::default()
        };
        assert_eq!(config.list_label_selector(), "app=sglang");
    }

    #[test]
    fn test_list_label_selector_pd_mode_no_common_labels() {
        let mut prefill = HashMap::new();
        prefill.insert("role".to_string(), "prefill".to_string());
        let mut decode = HashMap::new();
        decode.insert("role".to_string(), "decode".to_string());
        let config = ServiceDiscoveryConfig {
            pd_mode: true,
            prefill_selector: prefill,
            decode_selector: decode,
            ..Default::default()
        };
        assert!(config.list_label_selector().is_empty());
    }

    // ── Reconciliation tests ─────────────────────────────────────────────────

    #[test]
    fn test_build_live_pod_set_includes_matching() {
        let config = make_regular_config();
        let pods = vec![
            create_test_pod("pod-a", "10.0.0.1", "Running", "True",
                Some(vec![("app", "sglang")])),
            create_test_pod("pod-b", "10.0.0.2", "Running", "True",
                Some(vec![("app", "sglang")])),
        ];
        let live = build_live_pod_set(&pods, &config);
        assert_eq!(live.len(), 2);
    }

    #[test]
    fn test_build_live_pod_set_excludes_non_matching() {
        let config = make_regular_config();
        let pods = vec![
            create_test_pod("pod-a", "10.0.0.1", "Running", "True",
                Some(vec![("app", "sglang")])),
            create_test_pod("pod-b", "10.0.0.2", "Running", "True",
                Some(vec![("app", "other")])),
        ];
        let live = build_live_pod_set(&pods, &config);
        assert_eq!(live.len(), 1);
    }

    #[test]
    fn test_build_live_pod_set_excludes_deleted() {
        let config = make_regular_config();
        let mut deleted = create_test_pod("pod-a", "10.0.0.1", "Running", "True",
            Some(vec![("app", "sglang")]));
        deleted.metadata.deletion_timestamp =
            Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
                chrono::Utc::now(),
            ));
        let pods = vec![deleted];
        let live = build_live_pod_set(&pods, &config);
        assert!(live.is_empty());
    }

    #[test]
    fn test_compute_reconciliation_diff_no_changes() {
        let pod = make_pod_info("pod-a", "uid-a", "10.0.0.1", "Running", true);
        let tracked: HashSet<PodInfo> = [pod.clone()].into_iter().collect();
        let live: HashSet<PodInfo> = [pod].into_iter().collect();
        let (stale, missing) = compute_reconciliation_diff(&tracked, &live);
        assert!(stale.is_empty());
        assert!(missing.is_empty());
    }

    #[test]
    fn test_compute_reconciliation_diff_stale_pod() {
        let tracked_pod = make_pod_info("pod-a", "uid-a", "10.0.0.1", "Running", true);
        let tracked: HashSet<PodInfo> = [tracked_pod].into_iter().collect();
        let live: HashSet<PodInfo> = HashSet::new();
        let (stale, missing) = compute_reconciliation_diff(&tracked, &live);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].name, "pod-a");
        assert!(missing.is_empty());
    }

    #[test]
    fn test_compute_reconciliation_diff_missing_healthy() {
        let tracked: HashSet<PodInfo> = HashSet::new();
        let live_pod = make_pod_info("pod-b", "uid-b", "10.0.0.2", "Running", true);
        let live: HashSet<PodInfo> = [live_pod].into_iter().collect();
        let (stale, missing) = compute_reconciliation_diff(&tracked, &live);
        assert!(stale.is_empty());
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "pod-b");
    }

    #[test]
    fn test_compute_reconciliation_diff_missing_unhealthy_excluded() {
        let tracked: HashSet<PodInfo> = HashSet::new();
        let unhealthy = make_pod_info("pod-c", "uid-c", "10.0.0.3", "Running", false);
        let live: HashSet<PodInfo> = [unhealthy].into_iter().collect();
        let (stale, missing) = compute_reconciliation_diff(&tracked, &live);
        assert!(stale.is_empty());
        assert!(missing.is_empty());
    }

    #[test]
    fn test_compute_reconciliation_diff_mixed() {
        let pod_a = make_pod_info("pod-a", "uid-a", "10.0.0.1", "Running", true);
        let pod_b = make_pod_info("pod-b", "uid-b", "10.0.0.2", "Running", true);
        let pod_c = make_pod_info("pod-c", "uid-c", "10.0.0.3", "Running", true);
        let pod_d = make_pod_info("pod-d", "uid-d", "10.0.0.4", "Running", false);

        let tracked: HashSet<PodInfo> = [pod_a.clone(), pod_b.clone()].into_iter().collect();
        let live: HashSet<PodInfo> = [pod_b, pod_c.clone(), pod_d].into_iter().collect();

        let (stale, missing) = compute_reconciliation_diff(&tracked, &live);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].name, "pod-a");
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "pod-c");
    }

    #[test]
    fn test_reconciliation_detects_pod_restart() {
        let old = make_pod_info("worker-0", "uid-old", "10.0.0.1", "Running", true);
        let new = make_pod_info("worker-0", "uid-new", "10.0.0.1", "Running", true);
        let tracked: HashSet<PodInfo> = [old].into_iter().collect();
        let live: HashSet<PodInfo> = [new].into_iter().collect();
        let (stale, missing) = compute_reconciliation_diff(&tracked, &live);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].uid, "uid-old");
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].uid, "uid-new");
    }

    // ── Watcher config test ─────────────────────────────────────────────────

    #[test]
    fn test_build_watcher_config_with_selector() {
        let cfg = build_watcher_config("worker", "app=sglang");
        assert_eq!(cfg.label_selector.as_deref(), Some("app=sglang"));
    }

    #[test]
    fn test_build_watcher_config_empty_selector() {
        let cfg = build_watcher_config("worker", "");
        assert!(cfg.label_selector.is_none());
    }

    // ── Pool helper tests ───────────────────────────────────────────────────

    #[test]
    fn test_add_worker_to_pool() {
        let pool: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));
        assert!(add_worker_to_pool(&pool, "10.0.0.1:8000"));
        assert_eq!(pool.read().unwrap().len(), 1);
        // Duplicate should be rejected.
        assert!(!add_worker_to_pool(&pool, "10.0.0.1:8000"));
        assert_eq!(pool.read().unwrap().len(), 1);
    }

    #[test]
    fn test_remove_worker_from_pool() {
        let pool: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));
        add_worker_to_pool(&pool, "10.0.0.1:8000");
        add_worker_to_pool(&pool, "10.0.0.2:8000");
        assert!(remove_worker_from_pool(&pool, "10.0.0.1:8000"));
        assert_eq!(pool.read().unwrap().len(), 1);
        // Removing non-existent should return false.
        assert!(!remove_worker_from_pool(&pool, "10.0.0.3:8000"));
    }

    #[test]
    fn test_worker_url_format() {
        let info = make_pod_info("pod-1", "uid-1", "10.0.0.1", "Running", true);
        assert_eq!(info.worker_url(8000), "10.0.0.1:8000");
    }

    #[test]
    fn test_select_pool_regular_mode() {
        let regular: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));
        let info = make_pod_info("pod-1", "uid-1", "10.0.0.1", "Running", true);
        // In regular mode, should always return the regular pool
        let selected = select_pool(&info, false, &regular, None, None);
        assert!(Arc::ptr_eq(&selected, &regular));
    }

    #[test]
    fn test_select_pool_pd_mode_prefill() {
        let regular: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));
        let prefill: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));
        let decode: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));

        let mut info = make_pod_info("prefill-0", "uid-1", "10.0.0.1", "Running", true);
        info.pod_type = Some(PodType::Prefill);

        let selected = select_pool(&info, true, &regular, Some(&prefill), Some(&decode));
        assert!(Arc::ptr_eq(&selected, &prefill));
    }

    #[test]
    fn test_select_pool_pd_mode_decode() {
        let regular: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));
        let prefill: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));
        let decode: SharedWorkerPool = Arc::new(RwLock::new(Vec::new()));

        let mut info = make_pod_info("decode-0", "uid-1", "10.0.0.1", "Running", true);
        info.pod_type = Some(PodType::Decode);

        let selected = select_pool(&info, true, &regular, Some(&prefill), Some(&decode));
        assert!(Arc::ptr_eq(&selected, &decode));
    }
}
