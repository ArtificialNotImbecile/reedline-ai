use crate::{
    backend::Backend,
    context::{Context, validate_insertion},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Idle,
    Pending,
    Ready(String),
    Error(String),
}

#[derive(Clone, Copy)]
pub struct EngineConfig {
    pub debounce: Duration,
    pub cache_capacity: usize,
    pub cache_ttl: Duration,
}
impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(180),
            cache_capacity: 32,
            cache_ttl: Duration::from_secs(60),
        }
    }
}

struct Job {
    context: Context,
    generation: u64,
    due: Instant,
}
struct CacheEntry {
    context: Context,
    insertion: String,
    created: Instant,
}
struct State {
    context: Option<Context>,
    generation: u64,
    queued: Option<Job>,
    outcome: Outcome,
    changed: bool,
    shutdown: bool,
    cache: VecDeque<CacheEntry>,
    rendered_hint: Option<(Context, String, String)>,
    pending_hint_acceptance: Option<String>,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    repaint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    config: EngineConfig,
}
struct Worker {
    shared: Arc<Shared>,
    handle: Mutex<Option<JoinHandle<()>>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.state.lock().expect("state lock").shutdown = true;
        self.shared.wake.notify_all();
        // Shutdown waits at most the configured HTTP timeout for a running call.
        if let Some(handle) = self.handle.lock().expect("worker lock").take() {
            let _ = handle.join();
        }
    }
}

/// One worker, one in-flight request, and one replaceable queued snapshot.
/// Requests never block the editor and old generations cannot replace new ones.
#[derive(Clone)]
pub struct Engine {
    shared: Arc<Shared>,
    _worker: Arc<Worker>,
}
impl Engine {
    pub fn new(backend: Arc<dyn Backend>, config: EngineConfig) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                context: None,
                generation: 0,
                queued: None,
                outcome: Outcome::Idle,
                changed: false,
                shutdown: false,
                cache: VecDeque::new(),
                rendered_hint: None,
                pending_hint_acceptance: None,
            }),
            wake: Condvar::new(),
            repaint: Mutex::new(None),
            config: EngineConfig {
                cache_capacity: config.cache_capacity.min(128),
                ..config
            },
        });
        let worker_shared = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name("completion-worker".into())
            .spawn(move || run_worker(worker_shared, backend))
            .expect("could not start completion worker");
        let worker = Arc::new(Worker {
            shared: Arc::clone(&shared),
            handle: Mutex::new(Some(handle)),
        });
        Self {
            shared,
            _worker: worker,
        }
    }

    /// A rendered hint is a single-use token. The edit-mode parser snapshots
    /// acceptance in input order, before later batch edits. Reedline retains
    /// responsibility for applying the original native hint event.
    pub fn publish_rendered_hint(&self, context: &Context, text: &str) {
        let mut state = self.shared.state.lock().expect("state lock");
        if state.context.as_ref() == Some(context) && state.outcome == Outcome::Ready(text.into()) {
            state.rendered_hint = Some((context.clone(), text.into(), first_word(text)));
        }
    }

    pub fn invalidate_hint(&self) {
        self.shared.state.lock().expect("state lock").rendered_hint = None;
    }

    pub fn take_rendered_hint(&self) -> Option<String> {
        let mut state = self.shared.state.lock().expect("state lock");
        let (context, text, _) = state.rendered_hint.take()?;
        (state.context.as_ref() == Some(&context)).then_some(text)
    }

    pub fn take_rendered_hint_word(&self) -> Option<String> {
        let mut state = self.shared.state.lock().expect("state lock");
        let (context, _, word) = state.rendered_hint.take()?;
        (state.context.as_ref() == Some(&context)).then_some(word)
    }

    /// Local/native results win immediately; pending AI work becomes stale.
    pub fn prefer_local(&self, context: Context, text: &str, word: &str) {
        let mut state = self.shared.state.lock().expect("state lock");
        state.generation = state.generation.wrapping_add(1);
        state.context = Some(context.clone());
        state.queued = None;
        state.outcome = Outcome::Idle;
        state.changed = false;
        state.rendered_hint = (!text.is_empty()).then(|| (context, text.into(), word.into()));
        self.shared.wake.notify_one();
    }

    /// Preserve parsed input order, then let Reedline execute the ORIGINAL
    /// hint event with its native selection, menu, cursor, and undo guards.
    pub fn queue_hint_acceptance(&self, text: String) {
        self.shared
            .state
            .lock()
            .expect("state lock")
            .pending_hint_acceptance = Some(text);
    }
    pub fn consume_hint_acceptance(&self) -> String {
        self.shared
            .state
            .lock()
            .expect("state lock")
            .pending_hint_acceptance
            .take()
            .unwrap_or_default()
    }
    pub fn clear_hint_acceptance(&self) {
        self.shared
            .state
            .lock()
            .expect("state lock")
            .pending_hint_acceptance = None;
    }

    pub fn set_repaint(&self, repaint: impl Fn() + Send + Sync + 'static) {
        *self.shared.repaint.lock().expect("repaint lock") = Some(Arc::new(repaint));
    }

    pub fn request(&self, buffer: &str, cursor: usize) -> Outcome {
        self.request_validated(Context::new(buffer, cursor))
    }

    pub fn request_context(&self, context: Context) -> Outcome {
        self.request_validated(Ok(context))
    }

    fn request_validated(&self, context: Result<Context, &'static str>) -> Outcome {
        let mut state = self.shared.state.lock().expect("state lock");
        let context = match context {
            Ok(context) => context,
            Err(error) => {
                state.generation = state.generation.wrapping_add(1);
                state.context = None;
                state.rendered_hint = None;
                state.queued = None;
                state.outcome = Outcome::Error(error.into());
                self.shared.wake.notify_one();
                return state.outcome.clone();
            }
        };
        if state.context.as_ref() == Some(&context) {
            return state.outcome.clone();
        }
        state.generation = state.generation.wrapping_add(1);
        state.context = Some(context.clone());
        state.rendered_hint = None;
        state.queued = None;
        state.changed = false;
        let now = Instant::now();
        state
            .cache
            .retain(|entry| now.duration_since(entry.created) < self.shared.config.cache_ttl);
        state.outcome = if context.is_empty() {
            Outcome::Idle
        } else if let Some(index) = state
            .cache
            .iter()
            .position(|entry| entry.context == context)
        {
            let entry = state.cache.remove(index).expect("cache position");
            let insertion = entry.insertion.clone();
            state.cache.push_back(entry);
            Outcome::Ready(insertion)
        } else {
            state.queued = Some(Job {
                context,
                generation: state.generation,
                due: now + self.shared.config.debounce,
            });
            Outcome::Pending
        };
        self.shared.wake.notify_one();
        state.outcome.clone()
    }

    pub fn outcome(&self) -> Outcome {
        self.shared
            .state
            .lock()
            .expect("state lock")
            .outcome
            .clone()
    }

    /// Read only if this exact buffer and byte cursor remain current.
    pub fn outcome_for(&self, context: &Context) -> Outcome {
        let state = self.shared.state.lock().expect("state lock");
        if state.context.as_ref() == Some(context) {
            state.outcome.clone()
        } else {
            Outcome::Idle
        }
    }

    /// Returns (just finished, current request pending). Readiness is edge-triggered.
    pub fn poll(&self) -> (bool, bool) {
        let mut state = self.shared.state.lock().expect("state lock");
        (
            std::mem::take(&mut state.changed),
            matches!(state.outcome, Outcome::Pending),
        )
    }

    /// Invalidate queued/in-flight work after submit or cancellation, clearing all
    /// in-memory cached command text. An already-sent HTTP call cannot be recalled.
    pub fn clear(&self) {
        let mut state = self.shared.state.lock().expect("state lock");
        state.generation = state.generation.wrapping_add(1);
        state.context = None;
        state.rendered_hint = None;
        state.queued = None;
        state.outcome = Outcome::Idle;
        state.cache.clear();
        state.pending_hint_acceptance = None;
        state.changed = false;
        self.shared.wake.notify_one();
    }
}

fn first_word(text: &str) -> String {
    // Ask the native hinter for its Unicode-aware word boundary semantics.
    use reedline::{Hinter, History};
    let mut history = reedline::FileBackedHistory::new(1).expect("bounded history");
    let _ = history.save(reedline::HistoryItem::from_command_line(format!("x{text}")));
    let mut hinter = reedline::DefaultHinter::default();
    hinter.handle("x", 1, &history, false, "");
    hinter.next_hint_token()
}

fn run_worker(shared: Arc<Shared>, backend: Arc<dyn Backend>) {
    loop {
        let job = {
            let mut state = shared.state.lock().expect("state lock");
            loop {
                if state.shutdown {
                    return;
                }
                if let Some(job) = &state.queued {
                    let now = Instant::now();
                    if now >= job.due {
                        break state.queued.take().expect("queued job");
                    }
                    let duration = job.due - now;
                    state = shared
                        .wake
                        .wait_timeout(state, duration)
                        .expect("worker wait")
                        .0;
                } else {
                    state = shared.wake.wait(state).expect("worker wait");
                }
            }
        };
        let result = backend.complete(&job.context).and_then(|insertion| {
            validate_insertion(&insertion).map_err(crate::backend::BackendError::UnsafeOutput)?;
            Ok(insertion)
        });
        let should_repaint = {
            let mut state = shared.state.lock().expect("state lock");
            if state.shutdown {
                return;
            }
            if state.generation != job.generation || state.context.as_ref() != Some(&job.context) {
                // Do not cache stale work: clear() must not resurrect private input.
                false
            } else {
                state.outcome = match result {
                    Ok(insertion) => {
                        if shared.config.cache_capacity > 0 {
                            while state.cache.len() >= shared.config.cache_capacity {
                                state.cache.pop_front();
                            }
                            state.cache.push_back(CacheEntry {
                                context: job.context,
                                insertion: insertion.clone(),
                                created: Instant::now(),
                            });
                        }
                        Outcome::Ready(insertion)
                    }
                    Err(error) => Outcome::Error(error.to_string()),
                };
                state.changed = true;
                true
            }
        };
        if should_repaint {
            let repaint = shared.repaint.lock().expect("repaint lock").clone();
            if let Some(repaint) = repaint {
                repaint();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendError;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    fn fast_config() -> EngineConfig {
        EngineConfig {
            debounce: Duration::ZERO,
            ..EngineConfig::default()
        }
    }
    fn wait_ready(engine: &Engine) -> Outcome {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let outcome = engine.outcome();
            if !matches!(outcome, Outcome::Pending) {
                return outcome;
            }
            assert!(Instant::now() < deadline, "worker did not finish");
            thread::sleep(Duration::from_millis(2));
        }
    }
    struct CountingBackend(Arc<AtomicUsize>);
    impl Backend for CountingBackend {
        fn complete(&self, context: &Context) -> Result<String, BackendError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(format!("{}!", context.prefix()))
        }
    }
    #[test]
    fn debounce_coalesces_typing_and_cache_avoids_repeated_requests() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = Engine::new(
            Arc::new(CountingBackend(calls.clone())),
            EngineConfig {
                debounce: Duration::from_millis(50),
                ..fast_config()
            },
        );
        for line in ["g", "gi", "git"] {
            engine.request(line, line.len());
        }
        assert_eq!(wait_ready(&engine), Outcome::Ready("git!".into()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        engine.request("", 0);
        assert_eq!(engine.request("git", 3), Outcome::Ready("git!".into()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(engine.poll(), (false, false));
    }
    struct GatedBackend {
        started: mpsc::Sender<String>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl Backend for GatedBackend {
        fn complete(&self, context: &Context) -> Result<String, BackendError> {
            self.started.send(context.prefix().into()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            Ok(format!("{}!", context.prefix()))
        }
    }
    #[test]
    fn stale_inflight_result_cannot_overwrite_latest_snapshot() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let engine = Engine::new(
            Arc::new(GatedBackend {
                started: started_tx,
                release: Mutex::new(release_rx),
            }),
            fast_config(),
        );
        engine.request("old", 3);
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "old"
        );
        let start = Instant::now();
        engine.request("discard", 7);
        engine.request("new", 3);
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "editor blocked on backend"
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "new"
        );
        assert_eq!(engine.outcome(), Outcome::Pending);
        release_tx.send(()).unwrap();
        assert_eq!(wait_ready(&engine), Outcome::Ready("new!".into()));
        assert_eq!(engine.poll(), (true, false));
        assert_eq!(engine.poll(), (false, false));
        assert_eq!(
            engine.outcome_for(&Context::new("old", 3).unwrap()),
            Outcome::Idle
        );
    }
    #[test]
    fn clear_and_invalid_context_do_not_send_requests() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = Engine::new(
            Arc::new(CountingBackend(calls.clone())),
            EngineConfig {
                debounce: Duration::from_millis(40),
                ..fast_config()
            },
        );
        engine.request("private", 7);
        engine.clear();
        assert!(matches!(engine.request("世", 1), Outcome::Error(_)));
        thread::sleep(Duration::from_millis(70));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn whitespace_cancels_queued_requests_even_with_history_and_cursor_in_middle() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = Engine::new(
            Arc::new(CountingBackend(calls.clone())),
            EngineConfig {
                debounce: Duration::from_millis(40),
                ..fast_config()
            },
        );
        engine.request("git st", 6);
        let blank = Context::new("   ", 1)
            .unwrap()
            .with_history(vec!["git status".into()])
            .unwrap();
        assert_eq!(engine.request_context(blank), Outcome::Idle);
        thread::sleep(Duration::from_millis(70));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(engine.take_rendered_hint().is_none());
    }
    #[test]
    fn whitespace_discards_inflight_results_and_pending_queue() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let engine = Engine::new(
            Arc::new(GatedBackend {
                started: started_tx,
                release: Mutex::new(release_rx),
            }),
            fast_config(),
        );
        engine.request("old", 3);
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        engine.request("queued", 6);
        assert_eq!(engine.request("  ", 1), Outcome::Idle);
        release_tx.send(()).unwrap();
        thread::sleep(Duration::from_millis(30));
        assert_eq!(engine.outcome(), Outcome::Idle);
        assert!(started_rx.try_recv().is_err());
        assert!(engine.take_rendered_hint().is_none());
    }
    #[test]
    fn cache_is_bounded_and_expires() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = Engine::new(
            Arc::new(CountingBackend(calls.clone())),
            EngineConfig {
                cache_capacity: 1,
                cache_ttl: Duration::from_millis(20),
                ..fast_config()
            },
        );
        engine.request("a", 1);
        wait_ready(&engine);
        engine.request("b", 1);
        wait_ready(&engine);
        engine.request("a", 1);
        wait_ready(&engine);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        thread::sleep(Duration::from_millis(25));
        engine.request("", 0);
        engine.request("a", 1);
        wait_ready(&engine);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }
    struct UnsafeBackend;
    impl Backend for UnsafeBackend {
        fn complete(&self, _: &Context) -> Result<String, BackendError> {
            Ok("\x1b[2J".into())
        }
    }
    #[test]
    fn all_backends_are_subject_to_output_validation() {
        let engine = Engine::new(Arc::new(UnsafeBackend), fast_config());
        engine.request("a", 1);
        assert!(matches!(wait_ready(&engine), Outcome::Error(_)));
    }
}

#[cfg(test)]
mod local_race_tests {
    use super::*;
    use crate::backend::BackendError;
    use std::sync::mpsc;
    struct Gated {
        started: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl Backend for Gated {
        fn complete(&self, _: &Context) -> Result<String, BackendError> {
            self.started.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            Ok("AI result".into())
        }
    }
    #[test]
    fn inflight_ai_cannot_clobber_a_native_history_hint() {
        let (tx, rx) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let engine = Engine::new(
            Arc::new(Gated {
                started: tx,
                release: Mutex::new(gate),
            }),
            EngineConfig {
                debounce: Duration::ZERO,
                ..EngineConfig::default()
            },
        );
        engine.request("git ", 4);
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        engine.prefer_local(Context::new("git ", 4).unwrap(), "status --short", "status");
        release.send(()).unwrap();
        thread::sleep(Duration::from_millis(20));
        assert_eq!(engine.outcome(), Outcome::Idle);
        assert_eq!(engine.take_rendered_hint(), Some("status --short".into()));
        engine.clear();
        assert_eq!(engine.take_rendered_hint(), None);
    }
}
