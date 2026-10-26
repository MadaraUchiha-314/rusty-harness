//! The runtime seam.
//!
//! The core crate never names a concrete executor in a public signature. Code that has
//! to spawn a task or wait on a timer takes a [`Runtime`], and the caller picks an
//! implementation: [`TokioRuntime`] (feature `rt-tokio`) on native targets, or
//! `WasmRuntime` (feature `rt-wasm`) on `wasm32`.

use core::future::Future;
use core::pin::Pin;
use core::time::Duration;

/// `Send` on native targets, nothing on `wasm32`.
///
/// Browser futures (JS promises, timers) are `!Send`, and `wasm32-unknown-unknown` is
/// single-threaded, so bounds use `MaybeSend` instead of `Send` to stay satisfiable there.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}

/// `Send` on native targets, nothing on `wasm32`.
///
/// Browser futures (JS promises, timers) are `!Send`, and `wasm32-unknown-unknown` is
/// single-threaded, so bounds use `MaybeSend` instead of `Send` to stay satisfiable there.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}

#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// `Sync` on native targets, nothing on `wasm32`: the twin of [`MaybeSend`] for shared
/// references across the registry and the hook dispatcher.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync + ?Sized> MaybeSync for T {}

/// `Sync` on native targets, nothing on `wasm32`: the twin of [`MaybeSend`] for shared
/// references across the registry and the hook dispatcher.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}

#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSync for T {}

/// A monotonic instant: std's natively, `performance.now()` on `wasm32`, where
/// `std::time::Instant::now` panics.
pub use web_time::Instant;

/// The capabilities the harness needs from an async runtime.
pub trait Runtime {
    /// Runs `future` in the background, detached from the caller.
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + MaybeSend + 'static;

    /// Completes after `duration` has elapsed.
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + MaybeSend;
}

#[cfg(feature = "rt-tokio")]
pub use self::tokio::TokioRuntime;

#[cfg(all(feature = "rt-wasm", target_arch = "wasm32"))]
pub use self::wasm::WasmRuntime;

#[cfg(feature = "rt-tokio")]
mod tokio {
    use core::future::Future;
    use core::time::Duration;

    use super::{MaybeSend, Runtime};

    /// [`Runtime`] backed by a tokio runtime handle.
    #[derive(Clone, Debug)]
    pub struct TokioRuntime {
        handle: ::tokio::runtime::Handle,
    }

    impl TokioRuntime {
        /// Uses the tokio runtime the caller is running on.
        ///
        /// # Panics
        ///
        /// Panics when called outside a tokio runtime.
        #[must_use]
        pub fn current() -> Self {
            Self::from_handle(::tokio::runtime::Handle::current())
        }

        /// Uses the tokio runtime behind `handle`.
        #[must_use]
        pub fn from_handle(handle: ::tokio::runtime::Handle) -> Self {
            Self { handle }
        }
    }

    impl Runtime for TokioRuntime {
        fn spawn<F>(&self, future: F)
        where
            F: Future<Output = ()> + MaybeSend + 'static,
        {
            drop(self.handle.spawn(future));
        }

        fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + MaybeSend {
            let _guard = self.handle.enter();
            ::tokio::time::sleep(duration)
        }
    }

    #[cfg(test)]
    mod tests {
        use core::time::Duration;

        use super::{Runtime, TokioRuntime};

        #[tokio::test]
        async fn spawn_runs_the_future() {
            let (tx, rx) = ::tokio::sync::oneshot::channel();
            TokioRuntime::current().spawn(async move {
                tx.send(42).expect("receiver alive");
            });
            assert_eq!(rx.await.expect("task ran"), 42);
        }

        #[tokio::test(start_paused = true)]
        async fn sleep_waits_for_the_duration() {
            let start = ::tokio::time::Instant::now();
            TokioRuntime::current().sleep(Duration::from_secs(5)).await;
            assert!(start.elapsed() >= Duration::from_secs(5));
        }
    }
}

#[cfg(all(feature = "rt-wasm", target_arch = "wasm32"))]
mod wasm {
    use core::future::Future;
    use core::time::Duration;

    use super::{MaybeSend, Runtime};

    /// [`Runtime`] backed by the JS event loop: `spawn_local` and `setTimeout`.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct WasmRuntime;

    impl Runtime for WasmRuntime {
        fn spawn<F>(&self, future: F)
        where
            F: Future<Output = ()> + MaybeSend + 'static,
        {
            wasm_bindgen_futures::spawn_local(future);
        }

        fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + MaybeSend {
            gloo_timers::future::sleep(duration)
        }
    }
}

/// A boxed future that is `Send` on native targets and plain on `wasm32`.
///
/// A trait object may name only one non-auto trait, so this cannot be written with
/// [`MaybeSend`]; the two definitions are the same shape with `Send` added natively.
#[cfg(not(target_arch = "wasm32"))]
pub type HarnessFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A boxed future that is `Send` on native targets and plain on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub type HarnessFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// A boxed stream that is `Send` on native targets and plain on `wasm32`.
#[cfg(not(target_arch = "wasm32"))]
pub type HarnessStream<'a, T> = Pin<Box<dyn futures::Stream<Item = T> + Send + 'a>>;

/// A boxed stream that is `Send` on native targets and plain on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub type HarnessStream<'a, T> = Pin<Box<dyn futures::Stream<Item = T> + 'a>>;

/// `async_trait` natively and `async_trait(?Send)` on `wasm32`, for every dyn-compatible
/// async trait and its implementations.
///
/// Wrap the whole item: `harness_async_trait! { pub trait Clock { async fn now(&self); } }`.
#[macro_export]
macro_rules! harness_async_trait {
    ($($item:tt)*) => {
        #[cfg_attr(not(target_arch = "wasm32"), ::async_trait::async_trait)]
        #[cfg_attr(target_arch = "wasm32", ::async_trait::async_trait(?Send))]
        $($item)*
    };
}

/// Boxes a stream as a [`HarnessStream`]: `Send` natively, local on `wasm32`.
#[cfg(not(target_arch = "wasm32"))]
pub fn boxed_stream<'a, S>(stream: S) -> HarnessStream<'a, S::Item>
where
    S: futures::Stream + Send + 'a,
{
    Box::pin(stream)
}

/// Boxes a stream as a [`HarnessStream`]: `Send` natively, local on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub fn boxed_stream<'a, S>(stream: S) -> HarnessStream<'a, S::Item>
where
    S: futures::Stream + 'a,
{
    Box::pin(stream)
}
