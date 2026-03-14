//! Background task scheduler for deferred and periodic work (#95).
//!
//! Provides [`TaskScheduler`] which manages one-shot and recurring async tasks
//! using `tokio::spawn` and `tokio::time`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::debug;

/// Unique identifier for a scheduled task.
pub type TaskId = u64;

/// Internal state shared across clones.
struct SchedulerInner {
    next_id: AtomicU64,
    tasks: Mutex<HashMap<TaskId, TaskEntry>>,
}

struct TaskEntry {
    label: String,
    handle: JoinHandle<()>,
}

/// A scheduler for one-shot and periodic background tasks.
///
/// All tasks are cancelled when [`TaskScheduler::shutdown`] is called or when
/// the scheduler is dropped.
#[derive(Clone)]
pub struct TaskScheduler {
    inner: Arc<SchedulerInner>,
}

impl TaskScheduler {
    /// Create a new task scheduler.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                next_id: AtomicU64::new(1),
                tasks: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Schedule a one-shot task that executes after `delay`.
    ///
    /// Returns a [`TaskId`] that can be used to cancel the task.
    pub fn schedule_once<F, Fut>(&self, delay: Duration, label: impl Into<String>, f: F) -> TaskId
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let label_str = label.into();
        let inner = Arc::clone(&self.inner);
        let task_label = label_str.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            debug!("Running one-shot task {id} ({task_label})");
            f().await;
            // Remove self from the map after completion.
            inner.tasks.lock().await.remove(&id);
        });

        {
            let inner = Arc::clone(&self.inner);
            let label_str = label_str.clone();
            tokio::spawn(async move {
                inner.tasks.lock().await.insert(
                    id,
                    TaskEntry {
                        label: label_str,
                        handle,
                    },
                );
            });
        }

        id
    }

    /// Schedule a periodic task that runs every `interval`.
    ///
    /// The task function receives the current tick count (starting at 1).
    /// The first execution happens after `interval` elapses.
    ///
    /// Returns a [`TaskId`] that can be used to cancel the task.
    pub fn schedule_periodic<F, Fut>(
        &self,
        interval: Duration,
        label: impl Into<String>,
        f: F,
    ) -> TaskId
    where
        F: Fn(u64) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let label_str = label.into();
        let task_label = label_str.clone();

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // First tick fires immediately — skip it so the first execution
            // happens after one full interval.
            ticker.tick().await;

            let mut tick_count: u64 = 0;
            loop {
                ticker.tick().await;
                tick_count += 1;
                debug!("Periodic task {id} ({task_label}) tick {tick_count}");
                f(tick_count).await;
            }
        });

        let inner = Arc::clone(&self.inner);
        let label_owned = label_str;
        tokio::spawn(async move {
            inner.tasks.lock().await.insert(
                id,
                TaskEntry {
                    label: label_owned,
                    handle,
                },
            );
        });

        id
    }

    /// Cancel a previously scheduled task.
    ///
    /// Returns `true` if the task was found and cancelled.
    pub async fn cancel(&self, id: TaskId) -> bool {
        if let Some(entry) = self.inner.tasks.lock().await.remove(&id) {
            entry.handle.abort();
            debug!("Cancelled task {id} ({})", entry.label);
            true
        } else {
            false
        }
    }

    /// Return the number of active (not yet completed / cancelled) tasks.
    pub async fn active_count(&self) -> usize {
        self.inner.tasks.lock().await.len()
    }

    /// Cancel all tasks and shut down the scheduler.
    pub async fn shutdown(&self) {
        let mut tasks = self.inner.tasks.lock().await;
        for (id, entry) in tasks.drain() {
            entry.handle.abort();
            debug!("Shutdown: cancelled task {id} ({})", entry.label);
        }
    }
}

impl Default for TaskScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TaskScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskScheduler").finish()
    }
}

/// Convenience: schedule a one-shot closure that returns a boxed future.
pub fn boxed_task<F>(f: F) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>
where
    F: Future<Output = ()> + Send + 'static,
{
    Box::pin(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tokio::time;

    #[tokio::test]
    async fn test_schedule_once_executes() {
        let scheduler = TaskScheduler::new();
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);

        scheduler.schedule_once(Duration::from_millis(10), "test", move || {
            let f = Arc::clone(&flag_clone);
            async move {
                f.store(true, Ordering::SeqCst);
            }
        });

        time::sleep(Duration::from_millis(50)).await;
        assert!(flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_schedule_once_respects_delay() {
        let scheduler = TaskScheduler::new();
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);

        scheduler.schedule_once(Duration::from_millis(200), "delayed", move || {
            let f = Arc::clone(&flag_clone);
            async move {
                f.store(true, Ordering::SeqCst);
            }
        });

        // Should not have fired yet after 10ms.
        time::sleep(Duration::from_millis(10)).await;
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_schedule_periodic_fires_multiple_times() {
        let scheduler = TaskScheduler::new();
        let counter = Arc::new(AtomicU64::new(0));
        let counter_clone = Arc::clone(&counter);

        scheduler.schedule_periodic(Duration::from_millis(15), "ticker", move |_tick| {
            let c = Arc::clone(&counter_clone);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
            }
        });

        time::sleep(Duration::from_millis(100)).await;
        let count = counter.load(Ordering::SeqCst);
        // Should have fired multiple times (at least 3 in 100ms with 15ms interval).
        assert!(count >= 3, "expected >= 3 ticks, got {count}");
    }

    #[tokio::test]
    async fn test_cancel_task() {
        let scheduler = TaskScheduler::new();
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);

        let id = scheduler.schedule_once(Duration::from_millis(100), "cancel_me", move || {
            let f = Arc::clone(&flag_clone);
            async move {
                f.store(true, Ordering::SeqCst);
            }
        });

        // Give the spawn a moment to register.
        time::sleep(Duration::from_millis(5)).await;

        let cancelled = scheduler.cancel(id).await;
        assert!(cancelled);

        time::sleep(Duration::from_millis(150)).await;
        assert!(!flag.load(Ordering::SeqCst), "task should not have fired");
    }

    #[tokio::test]
    async fn test_shutdown_cancels_all() {
        let scheduler = TaskScheduler::new();
        let counter = Arc::new(AtomicU64::new(0));

        for i in 0..5 {
            let c = Arc::clone(&counter);
            scheduler.schedule_once(Duration::from_millis(200), format!("task-{i}"), move || {
                let c = Arc::clone(&c);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            });
        }

        time::sleep(Duration::from_millis(10)).await;
        scheduler.shutdown().await;

        time::sleep(Duration::from_millis(300)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_returns_false() {
        let scheduler = TaskScheduler::new();
        assert!(!scheduler.cancel(999).await);
    }

    #[test]
    fn test_default_scheduler() {
        let _scheduler = TaskScheduler::default();
    }

    #[test]
    fn test_debug_format() {
        let scheduler = TaskScheduler::new();
        let debug_str = format!("{:?}", scheduler);
        assert!(debug_str.contains("TaskScheduler"));
    }
}
