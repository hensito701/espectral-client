//! UI <-> engine bridge. FOUNDATION-OWNED.
//!
//! Rules (see UI-PORTING.md):
//! * The UI thread never awaits the engine. Every engine call goes through
//!   [`Ctx::call`] (or [`Ctx::call_with`]): the future runs on the shared tokio
//!   runtime and its result is applied on the UI thread with
//!   `slint::invoke_from_event_loop`, through a `Weak<AppWindow>` (a closed
//!   window simply drops the result).
//! * Errors become an "AVISO DEL SISTEMA" toast unless you handle them.
//! * Engine events (launch logs, progress, import-done, …) are delivered on
//!   the UI thread to handlers registered with [`on_event`], batched so a
//!   chatty game log costs one wake-up per batch.
//!
//! ```ignore
//! // in pages/<group>.rs, inside init(ctx):
//! let c = ctx.clone();
//! ui.global::<HomeBridge>().on_launch(move |name| {
//!     let name = name.to_string();
//!     c.call(
//!         move |e| async move { e.launch_instance(&name, LaunchRequest::default()).await },
//!         |ui, outcome| { /* runs on the UI thread */ },
//!     );
//! });
//! ```

// Public API for page modules: not every helper is used by the shell itself.
#![allow(dead_code)]

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use espectral_engine::events::EngineEvent;
use espectral_engine::{Engine, EngineError, EngineResult};
use slint::ComponentHandle;

use crate::AppWindow;

/// Shared context handed to every module's `init`. Cheap to clone.
#[derive(Clone)]
pub struct Ctx {
    inner: Arc<CtxInner>,
}

struct CtxInner {
    engine: Engine,
    rt: tokio::runtime::Handle,
    ui: slint::Weak<AppWindow>,
}

impl Ctx {
    pub fn new(engine: Engine, rt: tokio::runtime::Handle, ui: slint::Weak<AppWindow>) -> Self {
        Ctx { inner: Arc::new(CtxInner { engine, rt, ui }) }
    }

    /// The engine facade (clone it into futures: `Engine` is an `Arc`).
    pub fn engine(&self) -> &Engine {
        &self.inner.engine
    }

    /// Tokio runtime handle (for `spawn_blocking`, timers, …).
    pub fn rt(&self) -> &tokio::runtime::Handle {
        &self.inner.rt
    }

    /// Weak window handle (Send): upgrade it on the UI thread only.
    pub fn weak(&self) -> slint::Weak<AppWindow> {
        self.inner.ui.clone()
    }

    /// The window, when called on the UI thread and the window still exists.
    pub fn ui(&self) -> Option<AppWindow> {
        self.inner.ui.upgrade()
    }

    /// Spawn a raw future on the runtime (no result delivery).
    pub fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.inner.rt.spawn(fut);
    }

    /// Run `f` on the UI thread (from any thread). Dropped if the window is gone.
    pub fn on_ui<G>(&self, g: G)
    where
        G: FnOnce(&AppWindow) + Send + 'static,
    {
        let weak = self.weak();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                g(&ui);
            }
        });
    }

    /// Call the engine on the runtime; deliver `Ok` to `then` on the UI thread,
    /// turn `Err` into an error toast.
    pub fn call<T, Fut, F, G>(&self, f: F, then: G)
    where
        T: Send + 'static,
        Fut: Future<Output = EngineResult<T>> + Send + 'static,
        F: FnOnce(Engine) -> Fut + Send + 'static,
        G: FnOnce(&AppWindow, T) + Send + 'static,
    {
        self.call_with(f, then, |_ui, err| crate::toast::engine_error(&err));
    }

    /// Like [`Ctx::call`] with an explicit error handler (UI thread).
    pub fn call_with<T, Fut, F, G, H>(&self, f: F, then: G, on_err: H)
    where
        T: Send + 'static,
        Fut: Future<Output = EngineResult<T>> + Send + 'static,
        F: FnOnce(Engine) -> Fut + Send + 'static,
        G: FnOnce(&AppWindow, T) + Send + 'static,
        H: FnOnce(&AppWindow, EngineError) + Send + 'static,
    {
        let engine = self.inner.engine.clone();
        let weak = self.weak();
        self.inner.rt.spawn(async move {
            let res = f(engine).await;
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                match res {
                    Ok(v) => then(&ui, v),
                    Err(e) => {
                        tracing::warn!(error = %e, "engine call failed");
                        on_err(&ui, e)
                    }
                }
            });
        });
    }

    /// Fire-and-forget engine call: errors are logged + toasted, the value is dropped.
    pub fn fire<T, Fut, F>(&self, f: F)
    where
        T: Send + 'static,
        Fut: Future<Output = EngineResult<T>> + Send + 'static,
        F: FnOnce(Engine) -> Fut + Send + 'static,
    {
        self.call(f, |_ui, _v| {});
    }
}

// ---------------------------------------------------------------------------
// Engine events -> UI-thread handlers
// ---------------------------------------------------------------------------

type Handler = Rc<dyn Fn(&AppWindow, &EngineEvent)>;

thread_local! {
    static HANDLERS: RefCell<Vec<Handler>> = const { RefCell::new(Vec::new()) };
}

/// Register a handler for every engine event (UI thread; call it from your
/// module's `init`). Filter on the variant you care about:
///
/// ```ignore
/// bridge::on_event(|ui, ev| if let EngineEvent::ImportProgress(p) = ev { ... });
/// ```
pub fn on_event(f: impl Fn(&AppWindow, &EngineEvent) + 'static) {
    HANDLERS.with(|h| h.borrow_mut().push(Rc::new(f)));
}

fn dispatch(ui: &AppWindow, ev: &EngineEvent) {
    // clone the list so a handler may register another handler
    let handlers: Vec<Handler> = HANDLERS.with(|h| h.borrow().clone());
    for h in handlers {
        h(ui, ev);
    }
}

/// Subscribe to the engine broadcast and pump events to the UI thread in batches.
pub(crate) fn start_event_pump(ctx: &Ctx) {
    let mut rx = ctx.engine().subscribe();
    let weak = ctx.weak();
    let queue: Arc<Mutex<Vec<EngineEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let scheduled = Arc::new(AtomicBool::new(false));
    ctx.spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "UI event pump lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            queue.lock().unwrap_or_else(|p| p.into_inner()).push(ev);
            if !scheduled.swap(true, Ordering::AcqRel) {
                let q = queue.clone();
                let s = scheduled.clone();
                let w = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    s.store(false, Ordering::Release);
                    let batch = std::mem::take(&mut *q.lock().unwrap_or_else(|p| p.into_inner()));
                    if let Some(ui) = w.upgrade() {
                        for ev in &batch {
                            dispatch(&ui, ev);
                        }
                    }
                });
            }
        }
    });
}

/// Helper for page modules: `ui` upgrade in callbacks.
pub fn with_ui(weak: &slint::Weak<AppWindow>, f: impl FnOnce(&AppWindow)) {
    if let Some(ui) = weak.upgrade() {
        f(&ui);
    }
}

#[allow(dead_code)]
fn _assert_handle(ui: &AppWindow) -> slint::Weak<AppWindow> {
    ui.as_weak()
}
