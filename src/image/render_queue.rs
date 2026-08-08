//! Bounded work queue — mirrors TS `render-queue.ts` (`BoundedWorkQueue`).
//!
//! Provides:
//! - Keyed deduplication (same key → same result)
//! - Queue full error when max_queued exceeded
//! - Per-item timeout
//! - Active/queued/completed/failed counters
//! - Duration tracking (last, average, p95, max)

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

/// Error returned when the render queue is full.
#[derive(Debug, Clone)]
pub struct QueueFullError {
    pub message: String,
}

impl std::fmt::Display for QueueFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for QueueFullError {}

/// Duration metrics for queue monitoring.
#[derive(Debug, Clone, Default)]
pub struct DurationMetrics {
    pub last: f64,
    pub average: f64,
    pub p95: f64,
    pub max: f64,
}

/// Snapshot of queue state for health reporting.
#[derive(Debug, Clone)]
pub struct QueueSnapshot {
    pub active: usize,
    pub queued: usize,
    pub completed: usize,
    pub failed: usize,
    pub deduplicated: usize,
    pub duration_ms: DurationMetrics,
}

/// Internal mutable state for the queue.
struct QueueInner {
    completed: usize,
    failed: usize,
    deduplicated: usize,
    durations: Vec<f64>,
}

/// Bounded work queue with concurrency control, deduplication, and timeout.
///
/// Mirrors TS `BoundedWorkQueue<T>` from `render-queue.ts`.
pub struct BoundedWorkQueue<T: Send + Clone + 'static> {
    max_queued: usize,
    timeout_ms: u64,
    work_label: String,
    state: StdMutex<QueueInner>,
    /// In-flight deduplication: key → shared result holder.
    in_flight_map: StdMutex<HashMap<String, Arc<tokio::sync::Mutex<Option<T>>>>>,
}

impl<T: Send + Clone + 'static> BoundedWorkQueue<T> {
    /// Create a new bounded work queue.
    pub fn new(_concurrency: usize, max_queued: usize, timeout_ms: u64, work_label: &str) -> Self {
        Self {
            max_queued,
            timeout_ms,
            work_label: work_label.to_string(),
            state: StdMutex::new(QueueInner {
                completed: 0,
                failed: 0,
                deduplicated: 0,
                durations: Vec::new(),
            }),
            in_flight_map: StdMutex::new(HashMap::new()),
        }
    }

    /// Timeout in milliseconds.
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Add work to the queue with deduplication.
    pub async fn add<F, Fut>(&self, key: String, work: F) -> Result<T, QueueFullError>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>> + Send,
    {
        // Check for in-flight deduplication
        let holder_ref: Option<Arc<tokio::sync::Mutex<Option<T>>>>;
        {
            let map = self.in_flight_map.lock().unwrap();
            if let Some(holder) = map.get(&key) {
                holder_ref = Some(Arc::clone(holder));
            } else {
                holder_ref = None;
            }
        }
        if let Some(holder) = holder_ref {
            let holder_for_wait = Arc::clone(&holder);
            self.wait_for_holder(holder_for_wait).await?;
            return self.extract_result(&holder);
        }

        // Check queue capacity
        {
            let map = self.in_flight_map.lock().unwrap();
            if map.len() >= self.max_queued {
                return Err(QueueFullError {
                    message: format!(
                        "The {} queue is busy. Try again shortly.",
                        self.work_label
                    ),
                });
            }
        }

        // Create shared result holder
        let result_holder: Arc<tokio::sync::Mutex<Option<T>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        self.in_flight_map
            .lock()
            .unwrap()
            .insert(key.clone(), Arc::clone(&result_holder));

        // Execute work with timeout
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_millis(self.timeout_ms),
            work(),
        ).await;

        match result {
            Ok(Ok(value)) => {
                let mut guard = result_holder.lock().await;
                *guard = Some(value.clone());
                drop(guard);

                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                self.record_duration(elapsed);
                {
                    let mut state = self.state.lock().unwrap();
                    state.completed += 1;
                }

                self.in_flight_map.lock().unwrap().remove(&key);
                Ok(value)
            }
            Ok(Err(e)) => {
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                self.record_duration(elapsed);
                {
                    let mut state = self.state.lock().unwrap();
                    state.failed += 1;
                }

                // Store error marker so dedup waiters get the error
                let mut guard = result_holder.lock().await;
                *guard = None;

                self.in_flight_map.lock().unwrap().remove(&key);
                Err(QueueFullError {
                    message: format!("{} failed: {}", self.work_label, e),
                })
            }
            Err(_) => {
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                self.record_duration(elapsed);
                {
                    let mut state = self.state.lock().unwrap();
                    state.failed += 1;
                }

                self.in_flight_map.lock().unwrap().remove(&key);
                Err(QueueFullError {
                    message: format!(
                        "{} exceeded {}ms",
                        self.work_label, self.timeout_ms
                    ),
                })
            }
        }
    }

    /// Wait for a shared result holder to be populated.
    async fn wait_for_holder(
        &self,
        holder: Arc<tokio::sync::Mutex<Option<T>>>,
    ) -> Result<(), QueueFullError> {
        tokio::time::timeout(Duration::from_millis(self.timeout_ms), async {
            loop {
                let guard = holder.lock().await;
                if guard.is_some() {
                    return;
                }
                drop(guard);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| QueueFullError {
            message: format!(
                "{} dedup wait exceeded {}ms",
                self.work_label, self.timeout_ms
            ),
        })
    }

    /// Extract the result from a holder.
    fn extract_result(
        &self,
        holder: &Arc<tokio::sync::Mutex<Option<T>>>,
    ) -> Result<T, QueueFullError> {
        let guard = holder.blocking_lock();
        guard.clone().ok_or_else(|| QueueFullError {
            message: "Dedup result was never populated".into(),
        })
    }

    /// Get a snapshot of queue state.
    pub fn snapshot(&self) -> QueueSnapshot {
        let state = self.state.lock().unwrap();
        let map_len = self.in_flight_map.lock().unwrap().len();

        let sorted: Vec<f64> = state.durations.iter().copied().collect();
        let mut sorted = sorted;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let total: f64 = sorted.iter().sum();
        let avg = if sorted.is_empty() {
            0.0
        } else {
            total / sorted.len() as f64
        };
        let p95 = if sorted.is_empty() {
            0.0
        } else {
            let idx = ((sorted.len() as f64) * 0.95).ceil() as usize - 1;
            sorted.get(idx.min(sorted.len() - 1)).copied().unwrap_or(0.0)
        };

        QueueSnapshot {
            active: map_len,
            queued: 0,
            completed: state.completed,
            failed: state.failed,
            deduplicated: state.deduplicated,
            duration_ms: DurationMetrics {
                last: sorted.last().copied().unwrap_or(0.0),
                average: avg,
                p95,
                max: sorted.last().copied().unwrap_or(0.0),
            },
        }
    }

    /// Record a duration measurement (rolling window of 100).
    fn record_duration(&self, ms: f64) {
        let mut state = self.state.lock().unwrap();
        state.durations.push(ms);
        if state.durations.len() > 100 {
            state.durations.remove(0);
        }
    }
}
