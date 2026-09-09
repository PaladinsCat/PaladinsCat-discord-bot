//! Bound asynchronous work with keyed deduplication and execution timeouts.
//!
//! Limit active/queued producers, share duplicate results, and track success/failure counters.
//! Retain up to 100 duration samples for last/average/p95/max metrics; waiting for a permit has no execution timeout.
//! refs: doc: documents/05-operations/runbooks/discord-bot.md

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// Error returned when the render queue is full.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
#[derive(Debug, Clone)]
/// Carry a user-facing String for admission rejection, closed queue, producer error, or execution timeout.
/// Despite the name, this error also wraps failures after work has entered the queue.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct QueueFullError {
    pub message: String,
}

impl std::fmt::Display for QueueFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for QueueFullError {}

impl QueueFullError {
    /// True only when the producer's work timed out, not when the queue
    /// rejected admission.
    ///
    /// I/O: () -> `bool`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub fn is_work_timeout(&self) -> bool {
        self.message.contains(" exceeded ")
    }
}

/// Duration metrics for queue monitoring.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
#[derive(Debug, Clone, Default)]
/// Carry last, average, p95, and maximum duration values as f64 milliseconds.
/// The current snapshot computes both last and max from the largest retained sample; empty windows
/// yield zero.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct DurationMetrics {
    pub last: f64,
    pub average: f64,
    pub p95: f64,
    pub max: f64,
}

/// Snapshot of queue state for health reporting.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
#[derive(Debug, Clone)]
/// Report active/queued work, completed/failed producers, duplicate callers, and duration metrics.
/// Counters and sampled timings describe in-memory activity rather than durable job history.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct QueueSnapshot {
    pub active: usize,
    pub queued: usize,
    pub completed: usize,
    pub failed: usize,
    pub deduplicated: usize,
    pub duration_ms: DurationMetrics,
}

/// Internal mutable state for the queue.
/// refs: none
struct QueueInner {
    completed: usize,
    failed: usize,
    deduplicated: usize,
    durations: Vec<f64>,
}

/// Bounded work queue with concurrency control, deduplication, and timeout.
///
/// Mirrors TS `BoundedWorkQueue<T>` from `render-queue.ts`.
/// refs: doc: documents/05-operations/runbooks/discord-bot.md
pub struct BoundedWorkQueue<T: Send + Clone + 'static> {
    concurrency: usize,
    max_queued: usize,
    timeout_ms: u64,
    work_label: String,
    state: StdMutex<QueueInner>,
    permits: Arc<Semaphore>,
    active: AtomicUsize,
    /// In-flight deduplication: key → shared result holder.
    /// refs: none
    in_flight_map: StdMutex<HashMap<String, Arc<SharedResult<T>>>>,
}

/// One producer result shared by all duplicate callers. `Notify` prevents the
/// polling and runtime-blocking lock used by the first Rust implementation.
/// refs: none
struct SharedResult<T> {
    result: tokio::sync::Mutex<Option<Result<T, String>>>,
    ready: tokio::sync::Notify,
}

impl<T: Send + Clone + 'static> BoundedWorkQueue<T> {
    /// Create a new bounded work queue.
    ///
    /// Clamp concurrency to at least one and initialize permits/counters; no work is started.
    ///
    /// I/O: `usize` (concurrency), `usize` (max queued), `u64` (timeout ms), `&str` (work label) -> `BoundedWorkQueue<T>`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub fn new(concurrency: usize, max_queued: usize, timeout_ms: u64, work_label: &str) -> Self {
        let concurrency = concurrency.max(1);
        Self {
            concurrency,
            max_queued,
            timeout_ms,
            work_label: work_label.to_string(),
            state: StdMutex::new(QueueInner {
                completed: 0,
                failed: 0,
                deduplicated: 0,
                durations: Vec::new(),
            }),
            permits: Arc::new(Semaphore::new(concurrency)),
            active: AtomicUsize::new(0),
            in_flight_map: StdMutex::new(HashMap::new()),
        }
    }

    /// Add work to the queue with deduplication.
    ///
    /// Same-key callers share the producer result without invoking their closure. Reject a new key
    /// at concurrency+max_queued capacity; acquire a permit before starting its timeout. Work
    /// errors, closed permits, or timeout become QueueFullError; completion updates shared
    /// counters.
    ///
    /// I/O: `String` (key), `F: FnOnce() -> Fut + Send` (work), `Fut: Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>> + Send` -> `Result<T, QueueFullError>`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub async fn add<F, Fut>(&self, key: String, work: F) -> Result<T, QueueFullError>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>> + Send,
    {
        let (result_holder, producer) = {
            let mut map = self.in_flight_map.lock().unwrap();
            if let Some(existing) = map.get(&key) {
                self.state.lock().unwrap().deduplicated += 1;
                (Arc::clone(existing), false)
            } else {
                if map.len() >= self.concurrency + self.max_queued {
                    return Err(QueueFullError {
                        message: format!(
                            "The {} queue is busy. Try again shortly.",
                            self.work_label
                        ),
                    });
                }
                let holder = Arc::new(SharedResult {
                    result: tokio::sync::Mutex::new(None),
                    ready: tokio::sync::Notify::new(),
                });
                map.insert(key.clone(), Arc::clone(&holder));
                (holder, true)
            }
        };
        if !producer {
            return self.wait_for_result(result_holder).await;
        }

        // Waiting for a permit is queued work; its timeout starts only after
        // execution begins, matching the legacy TypeScript queue.
        let _permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| QueueFullError {
                message: format!("The {} queue is closed.", self.work_label),
            })?;
        self.active.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        let result = tokio::time::timeout(Duration::from_millis(self.timeout_ms), work()).await;
        self.active.fetch_sub(1, Ordering::Relaxed);

        let outcome = match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(format!("{} failed: {}", self.work_label, e)),
            Err(_) => Err(format!(
                "{} exceeded {}ms",
                self.work_label, self.timeout_ms
            )),
        };
        self.record_duration(started.elapsed().as_secs_f64() * 1000.0);
        {
            let mut state = self.state.lock().unwrap();
            if outcome.is_ok() {
                state.completed += 1;
            } else {
                state.failed += 1;
            }
        }
        *result_holder.result.lock().await = Some(outcome.clone());
        result_holder.ready.notify_waiters();
        self.in_flight_map.lock().unwrap().remove(&key);
        outcome.map_err(|message| QueueFullError { message })
    }

    async fn wait_for_result(&self, holder: Arc<SharedResult<T>>) -> Result<T, QueueFullError> {
        // This is the same logical request as the producer, so share its
        // bounded result instead of racing it with a second timeout clock.
        loop {
            let notified = holder.ready.notified();
            if let Some(result) = holder.result.lock().await.clone() {
                return result.map_err(|message| QueueFullError { message });
            }
            notified.await;
        }
    }

    /// Get a snapshot of queue state.
    ///
    /// Read mutex-protected counters and sort the retained duration window; empty metrics are zero
    /// and both last/max currently use the largest sample. Active is read separately, so this is
    /// approximate concurrent state.
    ///
    /// I/O: () -> `QueueSnapshot`
    /// refs: doc: documents/05-operations/runbooks/discord-bot.md
    pub fn snapshot(&self) -> QueueSnapshot {
        let state = self.state.lock().unwrap();
        let map_len = self.in_flight_map.lock().unwrap().len();

        let sorted: Vec<f64> = state.durations.to_vec();
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
            sorted
                .get(idx.min(sorted.len() - 1))
                .copied()
                .unwrap_or(0.0)
        };

        QueueSnapshot {
            active: self.active.load(Ordering::Relaxed),
            queued: map_len.saturating_sub(self.active.load(Ordering::Relaxed)),
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
    /// refs: none
    fn record_duration(&self, ms: f64) {
        let mut state = self.state.lock().unwrap();
        state.durations.push(ms);
        if state.durations.len() > 100 {
            state.durations.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounds_distinct_work_to_configured_concurrency() {
        let queue = Arc::new(BoundedWorkQueue::<u8>::new(1, 2, 200, "Render"));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for key in ["a", "b"] {
            let queue = Arc::clone(&queue);
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            tasks.push(tokio::spawn(async move {
                queue
                    .add(key.into(), move || async move {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(1)
                    })
                    .await
                    .unwrap()
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap(), 1);
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn deduplicates_success_and_returns_same_value() {
        let queue = Arc::new(BoundedWorkQueue::<u8>::new(1, 1, 200, "Render"));
        let calls = Arc::new(AtomicUsize::new(0));
        let first = {
            let queue = Arc::clone(&queue);
            let calls = Arc::clone(&calls);
            tokio::spawn(async move {
                queue
                    .add("same".into(), move || async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Ok(7)
                    })
                    .await
                    .unwrap()
            })
        };
        tokio::time::sleep(Duration::from_millis(2)).await;
        let second = queue.add("same".into(), || async { Ok(9) }).await.unwrap();
        assert_eq!(first.await.unwrap(), 7);
        assert_eq!(second, 7);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(queue.snapshot().deduplicated, 1);
    }

    #[tokio::test]
    async fn deduplicated_waiters_receive_producer_error() {
        let queue = Arc::new(BoundedWorkQueue::<u8>::new(1, 1, 200, "Render"));
        let first_queue = Arc::clone(&queue);
        let first = tokio::spawn(async move {
            first_queue
                .add("same".into(), || async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Err::<u8, Box<dyn std::error::Error + Send + Sync>>("boom".into())
                })
                .await
                .unwrap_err()
                .message
        });
        tokio::time::sleep(Duration::from_millis(2)).await;
        let second = queue
            .add("same".into(), || async { Ok(1) })
            .await
            .unwrap_err()
            .message;
        assert_eq!(first.await.unwrap(), second);
        assert!(second.contains("boom"));
    }

    #[tokio::test]
    async fn times_out_producer_and_duplicate_with_same_error() {
        let queue = Arc::new(BoundedWorkQueue::<u8>::new(1, 1, 20, "Render"));
        let first_queue = Arc::clone(&queue);
        let first = tokio::spawn(async move {
            first_queue
                .add("same".into(), || async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok(1)
                })
                .await
                .unwrap_err()
                .message
        });
        tokio::time::sleep(Duration::from_millis(2)).await;
        let second = queue
            .add("same".into(), || async { Ok(2) })
            .await
            .unwrap_err()
            .message;
        assert_eq!(first.await.unwrap(), second);
        assert!(second.contains("exceeded 20ms"));
    }
}
