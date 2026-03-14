//! Lazy initialization for expensive subsystems (#50).
//!
//! Uses `tokio::sync::OnceCell` to defer initialization of heavy subsystems
//! (LSP, MCP, embeddings) until first use rather than at startup.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use tokio::sync::OnceCell;
use tracing::debug;

/// A lazily-initialized subsystem value.
///
/// The inner `T` is initialized on the first call to [`LazySubsystem::get`]
/// or [`LazySubsystem::get_or_try_init`].  Subsequent calls return the
/// cached value without re-running the initializer.
///
/// This is a thin, ergonomic wrapper around `tokio::sync::OnceCell` that
/// adds logging and a human-readable subsystem name.
pub struct LazySubsystem<T: Send + Sync + 'static> {
    name: &'static str,
    cell: Arc<OnceCell<T>>,
}

impl<T: Send + Sync + 'static> LazySubsystem<T> {
    /// Create a new lazy subsystem with the given human-readable `name`.
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            cell: Arc::new(OnceCell::new()),
        }
    }

    /// Get the value, initializing it with `init` if necessary.
    ///
    /// The `init` future runs at most once, even under concurrent access.
    pub async fn get<F, Fut>(&self, init: F) -> &T
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        self.cell
            .get_or_init(|| async {
                debug!("Lazy-initializing subsystem: {}", self.name);
                let value = init().await;
                debug!("Subsystem {} initialized", self.name);
                value
            })
            .await
    }

    /// Get the value, initializing with a fallible `init` if necessary.
    ///
    /// If `init` returns an error, the cell remains uninitialized and future
    /// calls will retry.
    pub async fn get_or_try_init<F, Fut, E>(&self, init: F) -> Result<&T, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let name = self.name;
        self.cell
            .get_or_try_init(|| async {
                debug!("Lazy-initializing subsystem (fallible): {name}");
                let value = init().await?;
                debug!("Subsystem {name} initialized");
                Ok(value)
            })
            .await
    }

    /// Check whether the subsystem has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.cell.initialized()
    }

    /// Return the value if already initialized, without triggering init.
    pub fn try_get(&self) -> Option<&T> {
        self.cell.get()
    }

    /// The human-readable name of this subsystem.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

impl<T: Send + Sync + 'static> Clone for LazySubsystem<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name,
            cell: Arc::clone(&self.cell),
        }
    }
}

impl<T: Send + Sync + fmt::Debug + 'static> fmt::Debug for LazySubsystem<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LazySubsystem")
            .field("name", &self.name)
            .field("initialized", &self.is_initialized())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Convenience type aliases for the three named subsystems
// ---------------------------------------------------------------------------

/// Lazy-init wrapper for the LSP subsystem.
pub type LazyLsp<T> = LazySubsystem<T>;

/// Lazy-init wrapper for the MCP subsystem.
pub type LazyMcp<T> = LazySubsystem<T>;

/// Lazy-init wrapper for the embeddings subsystem.
pub type LazyEmbeddings<T> = LazySubsystem<T>;

/// Create standard lazy wrappers for LSP, MCP, and embeddings.
pub fn create_lazy_subsystems<L, M, E>() -> (LazyLsp<L>, LazyMcp<M>, LazyEmbeddings<E>)
where
    L: Send + Sync + 'static,
    M: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    (
        LazySubsystem::new("LSP"),
        LazySubsystem::new("MCP"),
        LazySubsystem::new("Embeddings"),
    )
}

// ---------------------------------------------------------------------------
// SyncLazy — for non-async contexts using std::sync::OnceLock
// ---------------------------------------------------------------------------

/// A synchronous lazy-init wrapper using `std::sync::OnceLock`.
///
/// Useful for subsystems that can be initialized without async.
pub struct SyncLazy<T: Send + Sync + 'static> {
    name: &'static str,
    cell: std::sync::OnceLock<T>,
}

impl<T: Send + Sync + 'static> SyncLazy<T> {
    /// Create a new synchronous lazy subsystem.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            cell: std::sync::OnceLock::new(),
        }
    }

    /// Get the value, initializing with `init` if necessary.
    pub fn get_or_init(&self, init: impl FnOnce() -> T) -> &T {
        self.cell.get_or_init(|| {
            debug!("Sync lazy-init: {}", self.name);
            init()
        })
    }

    /// Check whether initialized.
    pub fn is_initialized(&self) -> bool {
        self.cell.get().is_some()
    }

    /// Return the value if already initialized.
    pub fn try_get(&self) -> Option<&T> {
        self.cell.get()
    }

    /// The human-readable name.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

impl<T: Send + Sync + fmt::Debug + 'static> fmt::Debug for SyncLazy<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncLazy")
            .field("name", &self.name)
            .field("initialized", &self.is_initialized())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_lazy_init_on_first_use() {
        let lazy: LazySubsystem<String> = LazySubsystem::new("test");
        assert!(!lazy.is_initialized());
        assert!(lazy.try_get().is_none());

        let value = lazy.get(|| async { "hello".to_string() }).await;
        assert_eq!(value, "hello");
        assert!(lazy.is_initialized());
    }

    #[tokio::test]
    async fn test_lazy_init_only_once() {
        let counter = Arc::new(AtomicU32::new(0));
        let lazy: LazySubsystem<u32> = LazySubsystem::new("counter");

        let c1 = Arc::clone(&counter);
        let v1 = lazy
            .get(|| {
                let c = Arc::clone(&c1);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    42
                }
            })
            .await;

        let c2 = Arc::clone(&counter);
        let v2 = lazy
            .get(|| {
                let c = Arc::clone(&c2);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    99
                }
            })
            .await;

        assert_eq!(*v1, 42);
        assert_eq!(*v2, 42); // Same value, init not called again.
        assert_eq!(counter.load(Ordering::SeqCst), 1); // Init ran only once.
    }

    #[tokio::test]
    async fn test_lazy_fallible_init_success() {
        let lazy: LazySubsystem<String> = LazySubsystem::new("fallible");

        let result: Result<&String, &str> = lazy
            .get_or_try_init(|| async { Ok("success".to_string()) })
            .await;

        assert_eq!(result.unwrap(), "success");
        assert!(lazy.is_initialized());
    }

    #[tokio::test]
    async fn test_lazy_fallible_init_failure_allows_retry() {
        let lazy: LazySubsystem<String> = LazySubsystem::new("retry");
        let counter = Arc::new(AtomicU32::new(0));

        // First attempt fails.
        let c1 = Arc::clone(&counter);
        let result: Result<&String, String> = lazy
            .get_or_try_init(|| {
                let c = Arc::clone(&c1);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err("fail".to_string())
                }
            })
            .await;
        assert!(result.is_err());
        assert!(!lazy.is_initialized());

        // Second attempt succeeds.
        let c2 = Arc::clone(&counter);
        let result: Result<&String, String> = lazy
            .get_or_try_init(|| {
                let c = Arc::clone(&c2);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok("ok".to_string())
                }
            })
            .await;
        assert_eq!(result.unwrap(), "ok");
        assert!(lazy.is_initialized());
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_lazy_clone_shares_state() {
        let lazy1: LazySubsystem<u32> = LazySubsystem::new("shared");
        let lazy2 = lazy1.clone();

        let _ = lazy1.get(|| async { 100 }).await;
        assert!(lazy2.is_initialized());
        assert_eq!(*lazy2.try_get().unwrap(), 100);
    }

    #[tokio::test]
    async fn test_lazy_name() {
        let lazy: LazySubsystem<()> = LazySubsystem::new("MySubsystem");
        assert_eq!(lazy.name(), "MySubsystem");
    }

    #[tokio::test]
    async fn test_create_lazy_subsystems() {
        let (lsp, mcp, emb) = create_lazy_subsystems::<String, String, String>();
        assert_eq!(lsp.name(), "LSP");
        assert_eq!(mcp.name(), "MCP");
        assert_eq!(emb.name(), "Embeddings");
        assert!(!lsp.is_initialized());
        assert!(!mcp.is_initialized());
        assert!(!emb.is_initialized());
    }

    #[test]
    fn test_sync_lazy_init() {
        let lazy = SyncLazy::new("sync-test");
        assert!(!lazy.is_initialized());

        let val = lazy.get_or_init(|| 42u32);
        assert_eq!(*val, 42);
        assert!(lazy.is_initialized());

        // Second call returns same value.
        let val2 = lazy.get_or_init(|| 99);
        assert_eq!(*val2, 42);
    }

    #[test]
    fn test_sync_lazy_try_get() {
        let lazy = SyncLazy::new("sync-try");
        assert!(lazy.try_get().is_none());

        lazy.get_or_init(|| "hello");
        assert_eq!(*lazy.try_get().unwrap(), "hello");
    }

    #[test]
    fn test_sync_lazy_name() {
        let lazy = SyncLazy::<()>::new("test-name");
        assert_eq!(lazy.name(), "test-name");
    }

    #[tokio::test]
    async fn test_lazy_debug_format() {
        let lazy: LazySubsystem<u32> = LazySubsystem::new("dbg");
        let debug_str = format!("{:?}", lazy);
        assert!(debug_str.contains("LazySubsystem"));
        assert!(debug_str.contains("dbg"));
        assert!(debug_str.contains("false"));

        let _ = lazy.get(|| async { 1 }).await;
        let debug_str = format!("{:?}", lazy);
        assert!(debug_str.contains("true"));
    }

    #[test]
    fn test_sync_lazy_debug_format() {
        let lazy = SyncLazy::<u32>::new("sync-dbg");
        let debug_str = format!("{:?}", lazy);
        assert!(debug_str.contains("SyncLazy"));
        assert!(debug_str.contains("sync-dbg"));
    }
}
