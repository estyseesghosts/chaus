//! The single owner of runtime settings disk interaction.
//!
//! `SettingsGlobal` stays the authoritative live value. This entity owns only
//! the path, diagnostics and serialized disk state: one operation at a time,
//! newest pending snapshot wins, every result carries a generation, and an
//! explicit Reload or shutdown establishes a barrier that old work cannot
//! cross.

use std::io;
use std::path::{Path, PathBuf};

use chaus_core::settings::Settings;
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use silvestris::ThemePreference;

use crate::app::apply_to_open_editors;
use crate::settings_file::{self, SettingsRead};
use crate::settings_global::SettingsGlobal;
use crate::theme_choice::theme_id;

/// Whether a write is an ordinary preference change or the final exit write.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    Normal,
    Final,
}

/// One disk write handed to the worker.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct WriteRequest {
    pub generation: u64,
    pub settings: Settings,
    pub kind: WriteKind,
}

/// What the caller must do after `SettingsPersistence::complete_write`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum WriteCompletion {
    /// Publish the new status; no new operation starts.
    Notify,
    /// The old write settled, so the reload read may start.
    BeginRead { epoch: u64 },
    /// The old write settled, so the final snapshot may be committed.
    BeginFinalWrite(WriteRequest),
    /// Shutdown is serialized and complete.
    ShutdownFinished,
}

/// Result of `SettingsPersistence::begin_reload`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadStart {
    ReadNow { epoch: u64 },
    WaitForWrite { epoch: u64, generation: u64 },
    Rejected,
}

/// Result of `SettingsPersistence::complete_reload`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ReloadOutcome {
    Accepted(SettingsRead),
    Rejected(String),
}

/// Result of `SettingsPersistence::begin_shutdown`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ShutdownStart {
    WriteFinal(WriteRequest),
    WaitForWrite { generation: u64 },
    AlreadyFinished,
}

/// Next operation the entity should start.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum PersistenceAction {
    Write(WriteRequest),
    Read { epoch: u64 },
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReloadState {
    Idle,
    AwaitingWrite { epoch: u64, generation: u64 },
    Reading { epoch: u64, live_generation: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShutdownState {
    Running,
    Closing { barrier: u64 },
    WritingFinal { generation: u64 },
    Finished,
}

/// Coalescing write state, kept independent of I/O and GPUI.
#[derive(Default)]
pub struct WriteQueue {
    next_generation: u64,
    in_flight: Option<u64>,
    pending: Option<(u64, Settings)>,
    /// Newest snapshot acknowledged in memory, even while a write is running.
    latest: Option<Settings>,
    error: Option<(u64, String)>,
    completed: u64,
    /// Generations at or below this barrier belong to an abandoned epoch.
    barrier: u64,
}

impl WriteQueue {
    /// Accept a snapshot and return its generation.
    pub fn enqueue(&mut self, settings: Settings) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.latest = Some(settings.clone());
        self.pending = Some((self.next_generation, settings));
        self.next_generation
    }

    /// Hand the newest pending snapshot to an idle worker.
    pub fn take(&mut self) -> Option<(u64, Settings)> {
        if self.in_flight.is_some() {
            return None;
        }
        let (generation, settings) = self.pending.take()?;
        self.in_flight = Some(generation);
        Some((generation, settings))
    }

    /// Record a finished write. A stale generation is ignored so a failed old
    /// attempt cannot clear a newer success or overwrite a newer error.
    pub fn finish(&mut self, generation: u64, result: io::Result<()>) {
        if self.in_flight != Some(generation) {
            return;
        }
        self.in_flight = None;
        if generation <= self.barrier {
            // The writer started before a reload/shutdown barrier. It may
            // still have touched the file, but its status is not current.
            return;
        }
        match result {
            Ok(()) => {
                self.completed = self.completed.max(generation);
                if self
                    .error
                    .as_ref()
                    .is_some_and(|(failed, _)| *failed <= generation)
                {
                    self.error = None;
                }
            }
            Err(error) => self.error = Some((generation, error.to_string())),
        }
    }

    /// The error that should stay visible until a newer write succeeds.
    #[must_use]
    pub fn visible_error(&self) -> Option<&str> {
        self.error.as_ref().and_then(|(generation, message)| {
            (*generation > self.completed).then_some(message.as_str())
        })
    }

    #[must_use]
    pub fn has_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Newest acknowledged snapshot, for the serialized exit write.
    #[must_use]
    pub fn latest(&self) -> Option<&Settings> {
        self.latest.as_ref()
    }

    /// Abandon not-yet-started writes and return the generation barrier.
    pub fn abandon_older(&mut self) -> u64 {
        self.barrier = self.next_generation;
        self.pending = None;
        self.barrier
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    pub fn set_latest(&mut self, settings: Settings) {
        self.latest = Some(settings);
    }

    #[must_use]
    pub fn in_flight_generation(&self) -> Option<u64> {
        self.in_flight
    }

    #[must_use]
    pub fn next_generation(&self) -> u64 {
        self.next_generation
    }

    /// Reserve a generation for the exit write without starting it yet.
    pub fn reserve_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }

    pub fn mark_in_flight(&mut self, generation: u64) {
        self.in_flight = Some(generation);
    }
}

/// Pure persistence state machine. All transitions are explicit so races can
/// be tested without threads, timers or sleeps.
pub struct SettingsPersistence {
    queue: WriteQueue,
    path: Option<PathBuf>,
    diagnostics: Vec<String>,
    reload_epoch: u64,
    reload: ReloadState,
    read_started: bool,
    shutdown: ShutdownState,
    final_request: Option<WriteRequest>,
}

impl SettingsPersistence {
    #[must_use]
    pub fn new(path: Option<PathBuf>, diagnostics: Vec<String>) -> Self {
        Self {
            queue: WriteQueue::default(),
            path,
            diagnostics,
            reload_epoch: 0,
            reload: ReloadState::Idle,
            read_started: false,
            shutdown: ShutdownState::Running,
            final_request: None,
        }
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    #[must_use]
    pub fn visible_error(&self) -> Option<&str> {
        self.queue.visible_error()
    }

    #[must_use]
    pub fn latest(&self) -> Option<&Settings> {
        self.queue.latest()
    }

    #[must_use]
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown != ShutdownState::Running
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.shutdown == ShutdownState::Finished
    }

    /// Accept a live snapshot. Returns `None` after shutdown begins.
    pub fn enqueue(&mut self, settings: Settings) -> Option<u64> {
        if self.is_shutting_down() {
            return None;
        }
        Some(self.queue.enqueue(settings))
    }

    /// Take the next write, if any. Ordinary writes stop at a reload or
    /// shutdown barrier; the exit write is released only after older work.
    pub fn take_write(&mut self) -> Option<WriteRequest> {
        if let Some(request) = self.final_request.take() {
            self.queue.mark_in_flight(request.generation);
            return Some(request);
        }
        if self.is_shutting_down() || matches!(self.reload, ReloadState::AwaitingWrite { .. }) {
            return None;
        }
        let (generation, settings) = self.queue.take()?;
        Some(WriteRequest {
            generation,
            settings,
            kind: WriteKind::Normal,
        })
    }

    /// Record an operation's completion and advance the state machine.
    pub fn complete_write(&mut self, generation: u64, result: io::Result<()>) -> WriteCompletion {
        let final_generation = match self.shutdown {
            ShutdownState::WritingFinal { generation } => Some(generation),
            _ => None,
        };
        self.queue.finish(generation, result);

        if final_generation == Some(generation) {
            self.shutdown = ShutdownState::Finished;
            return WriteCompletion::ShutdownFinished;
        }

        if let ShutdownState::Closing { barrier } = self.shutdown {
            if generation <= barrier && !self.queue.has_in_flight() {
                return match self.queue_final() {
                    ShutdownStart::WriteFinal(request) => WriteCompletion::BeginFinalWrite(request),
                    _ => WriteCompletion::ShutdownFinished,
                };
            }
        }

        if let ReloadState::AwaitingWrite {
            epoch,
            generation: waiting,
        } = self.reload
        {
            if generation == waiting && !self.queue.has_in_flight() {
                self.reload = ReloadState::Reading {
                    epoch,
                    live_generation: self.queue.next_generation(),
                };
                self.read_started = false;
                return WriteCompletion::BeginRead { epoch };
            }
        }

        WriteCompletion::Notify
    }

    /// Establish a reload barrier. Pending writes from the old UI state are
    /// abandoned; an already-running write must settle before the read.
    pub fn begin_reload(&mut self) -> ReloadStart {
        if self.is_shutting_down() {
            return ReloadStart::Rejected;
        }
        self.reload_epoch = self.reload_epoch.wrapping_add(1);
        let epoch = self.reload_epoch;
        self.queue.abandon_older();
        self.queue.clear_error();
        self.read_started = false;
        match self.queue.in_flight_generation() {
            Some(generation) => {
                self.reload = ReloadState::AwaitingWrite { epoch, generation };
                ReloadStart::WaitForWrite { epoch, generation }
            }
            None => {
                self.reload = ReloadState::Reading {
                    epoch,
                    live_generation: self.queue.next_generation(),
                };
                ReloadStart::ReadNow { epoch }
            }
        }
    }

    /// Accept a read result only if its epoch and live generation are current.
    pub fn complete_reload(
        &mut self,
        epoch: u64,
        outcome: Result<SettingsRead, String>,
    ) -> ReloadOutcome {
        if self.is_shutting_down() {
            return ReloadOutcome::Rejected("settings are shutting down".to_owned());
        }
        let ReloadState::Reading {
            epoch: expected,
            live_generation,
        } = self.reload
        else {
            return ReloadOutcome::Rejected("reload was superseded".to_owned());
        };
        if epoch != expected || self.queue.next_generation() != live_generation {
            return ReloadOutcome::Rejected("settings changed while reloading".to_owned());
        }
        self.reload = ReloadState::Idle;
        self.read_started = false;
        match outcome {
            Ok(read) => {
                self.diagnostics = read.diagnostics.clone();
                self.queue.set_latest(read.settings.clone());
                ReloadOutcome::Accepted(read)
            }
            Err(message) => {
                self.diagnostics = vec![message.clone()];
                ReloadOutcome::Rejected(message)
            }
        }
    }

    /// Stop accepting requests and serialize the exit write behind older work.
    pub fn begin_shutdown(&mut self) -> ShutdownStart {
        if self.is_shutting_down() {
            return ShutdownStart::AlreadyFinished;
        }
        let barrier = self.queue.abandon_older();
        self.shutdown = ShutdownState::Closing { barrier };
        match self.queue.in_flight_generation() {
            Some(generation) => ShutdownStart::WaitForWrite { generation },
            None => self.queue_final(),
        }
    }

    fn queue_final(&mut self) -> ShutdownStart {
        let Some(settings) = self.queue.latest().cloned() else {
            self.shutdown = ShutdownState::Finished;
            return ShutdownStart::AlreadyFinished;
        };
        let generation = self.queue.reserve_generation();
        let request = WriteRequest {
            generation,
            settings,
            kind: WriteKind::Final,
        };
        self.final_request = Some(request.clone());
        self.shutdown = ShutdownState::WritingFinal { generation };
        ShutdownStart::WriteFinal(request)
    }

    /// The next operation the entity should start, with its state reserved.
    pub fn next_action(&mut self) -> PersistenceAction {
        if let Some(request) = self.take_write() {
            return PersistenceAction::Write(request);
        }
        if let ReloadState::Reading { epoch, .. } = self.reload {
            if !self.read_started {
                self.read_started = true;
                return PersistenceAction::Read { epoch };
            }
        }
        PersistenceAction::None
    }
}

/// Status changes published to subscribers such as Preferences.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsStoreEvent {
    /// A write completed, a reload was requested, or shutdown advanced.
    Changed,
    /// A generation-checked reload was accepted; live settings are updated.
    Reloaded(Settings),
}

/// The app-wide persistence entity.
pub struct SettingsStore {
    state: SettingsPersistence,
    /// The single running operation. Held so shutdown can await it instead of
    /// polling or sleeping.
    active: Option<Task<()>>,
}

impl EventEmitter<SettingsStoreEvent> for SettingsStore {}

/// One store per app, reachable from Preferences and the exit path.
pub struct SettingsStoreGlobal(pub Entity<SettingsStore>);

impl Global for SettingsStoreGlobal {}

impl SettingsStore {
    #[must_use]
    pub fn new(path: Option<PathBuf>, diagnostics: Vec<String>) -> Self {
        Self {
            state: SettingsPersistence::new(path, diagnostics),
            active: None,
        }
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.state.path()
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[String] {
        self.state.diagnostics()
    }

    #[must_use]
    pub fn visible_error(&self) -> Option<String> {
        self.state.visible_error().map(str::to_owned)
    }

    #[must_use]
    pub fn state(&self) -> &SettingsPersistence {
        &self.state
    }

    /// Queue a live snapshot. Disk work is independent of the caller.
    pub fn enqueue(&mut self, settings: Settings, cx: &mut Context<Self>) {
        if self.state.enqueue(settings).is_some() {
            self.kick(cx);
            cx.emit(SettingsStoreEvent::Changed);
            cx.notify();
        }
    }

    /// Explicit disk-as-source-of-truth request.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.state.begin_reload() != ReloadStart::Rejected {
            self.kick(cx);
            cx.emit(SettingsStoreEvent::Changed);
            cx.notify();
        }
    }

    /// Serialized exit path: stop accepting requests, let older work settle,
    /// then write the newest accepted snapshot exactly once.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if self.state.begin_shutdown() == ShutdownStart::AlreadyFinished {
            return Task::ready(());
        }
        self.kick(cx);
        let this = cx.entity().clone();
        cx.spawn(async move |_, cx| {
            loop {
                let active = this.update(cx, |store, _| store.active.take());
                if let Some(active) = active {
                    active.await;
                    continue;
                }
                if this.update(cx, |store, _| store.state.is_finished()) {
                    break;
                }
                // No task is running yet; start the final write and await it.
                let started = this.update(cx, |store, cx| {
                    store.kick(cx);
                    store.active.is_some()
                });
                if !started {
                    break;
                }
            }
            this.update(cx, |_, cx| {
                cx.emit(SettingsStoreEvent::Changed);
                cx.notify();
            });
        })
    }

    fn kick(&mut self, cx: &mut Context<Self>) {
        if self.active.is_some() {
            return;
        }
        match self.state.next_action() {
            PersistenceAction::Write(request) => self.spawn_write(request, cx),
            PersistenceAction::Read { epoch } => self.spawn_read(epoch, cx),
            PersistenceAction::None => {}
        }
    }

    fn spawn_write(&mut self, request: WriteRequest, cx: &mut Context<Self>) {
        let path = self.state.path().map(Path::to_path_buf);
        let generation = request.generation;
        let task = cx.background_executor().spawn(async move {
            match path {
                Some(path) => settings_file::write_rc(&path, &request.settings),
                None => Ok(()),
            }
        });
        let this = cx.entity().clone();
        self.active = Some(cx.spawn(async move |_, cx| {
            let result = task.await;
            this.update(cx, |store, cx| {
                store.active = None;
                // The state machine may have released a reload read or the
                // final exit write at this point; `kick` starts it.
                let _ = store.state.complete_write(generation, result);
                store.kick(cx);
                cx.emit(SettingsStoreEvent::Changed);
                cx.notify();
            });
        }));
    }

    fn spawn_read(&mut self, epoch: u64, cx: &mut Context<Self>) {
        let path = self.state.path().map(Path::to_path_buf);
        let task = cx
            .background_executor()
            .spawn(async move { settings_file::read_settings_in(path.as_deref()) });
        let this = cx.entity().clone();
        self.active = Some(cx.spawn(async move |_, cx| {
            let read = task.await;
            this.update(cx, |store, cx| {
                store.active = None;
                if let ReloadOutcome::Accepted(read) = store.state.complete_reload(epoch, Ok(read))
                {
                    apply_live(&read.settings, cx);
                    cx.emit(SettingsStoreEvent::Reloaded(read.settings));
                }
                store.kick(cx);
                cx.emit(SettingsStoreEvent::Changed);
                cx.notify();
            });
        }));
    }
}

/// Apply a generation-checked reload to all open windows.
fn apply_live(settings: &Settings, cx: &mut App) {
    cx.set_global(SettingsGlobal(settings.clone()));
    silvestris::set_theme_preference(ThemePreference::explicit(theme_id(settings.theme)), cx);
    apply_to_open_editors(settings, cx);
}

/// Get or install the single store, preserving the startup read result.
pub(crate) fn install_store(cx: &mut App, path: Option<PathBuf>, diagnostics: Vec<String>) {
    if cx.try_global::<SettingsStoreGlobal>().is_none() {
        let store = cx.new(|_| SettingsStore::new(path, diagnostics));
        cx.set_global(SettingsStoreGlobal(store));
    }
}

/// Lazily install a store for callers that open Preferences without `app::run`.
pub(crate) fn store(cx: &mut App) -> Entity<SettingsStore> {
    if let Some(global) = cx.try_global::<SettingsStoreGlobal>() {
        return global.0.clone();
    }
    let read = settings_file::read_settings();
    install_store(cx, read.path, read.diagnostics);
    cx.global::<SettingsStoreGlobal>().0.clone()
}

/// Narrow seam for integration tests: install a store with a chosen path.
#[doc(hidden)]
pub fn install_store_for_test(
    cx: &mut App,
    path: Option<PathBuf>,
    diagnostics: Vec<String>,
) -> Entity<SettingsStore> {
    let store = cx.new(|_| SettingsStore::new(path, diagnostics));
    cx.set_global(SettingsStoreGlobal(store.clone()));
    store
}
