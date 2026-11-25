//! Serialized, coalescing persistence for the Preferences rc file.
//!
//! A preference event applies to memory at once; disk work is queued here. Only
//! one write runs at a time, and a newer event replaces the pending snapshot
//! instead of starting a competing writer. Errors belong to a generation and
//! stay visible until a newer write succeeds.

use std::path::PathBuf;

use chaus_core::settings::Settings;
use gpui::{App, AppContext as _, Context, Entity, Global};

/// Pure queue state. Kept free of I/O and GPUI so coalescing and generation
/// binding can be tested deterministically by completing writes out of order.
#[derive(Default)]
pub struct WriteQueue {
    next_generation: u64,
    in_flight: Option<u64>,
    pending: Option<(u64, Settings)>,
    /// Newest snapshot acknowledged in memory, even while a write is in flight.
    latest: Option<Settings>,
    error: Option<(u64, String)>,
    completed: u64,
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
    pub fn finish(&mut self, generation: u64, result: std::io::Result<()>) {
        if self.in_flight != Some(generation) {
            return;
        }
        self.in_flight = None;
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
    pub fn visible_error(&self) -> Option<&str> {
        self.error.as_ref().and_then(|(generation, message)| {
            (*generation > self.completed).then_some(message.as_str())
        })
    }

    pub fn has_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Newest acknowledged snapshot, for the synchronous exit flush.
    pub fn latest(&self) -> Option<&Settings> {
        self.latest.as_ref()
    }
}

/// The app-wide writer entity.
pub(crate) struct SettingsWriter {
    queue: WriteQueue,
    path: Option<PathBuf>,
}

/// One writer per app, reachable from the exit path.
pub(crate) struct SettingsWriterGlobal(pub(crate) Entity<SettingsWriter>);

impl Global for SettingsWriterGlobal {}

impl SettingsWriter {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            queue: WriteQueue::default(),
            path,
        }
    }

    /// Queue a snapshot; persistence runs independently of the caller.
    pub fn enqueue(&mut self, settings: Settings, cx: &mut Context<Self>) {
        self.queue.enqueue(settings);
        self.kick(cx);
    }

    /// Start the next queued write when the worker is idle.
    fn kick(&mut self, cx: &mut Context<Self>) {
        let Some((generation, settings)) = self.queue.take() else {
            return;
        };
        let path = self.path.clone();
        let task = cx.background_executor().spawn(async move {
            match path {
                Some(path) => chaus_io::write_settings_rc(&path, &settings),
                None => Ok(()),
            }
        });
        let this = cx.entity().clone();
        cx.spawn(async move |_, cx| {
            let result = task.await;
            this.update(cx, |writer, cx| {
                writer.queue.finish(generation, result);
                writer.kick(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn visible_error(&self) -> Option<String> {
        self.queue.visible_error().map(str::to_owned)
    }

    /// Final flush at process exit. Writing the newest acknowledged snapshot
    /// synchronously guarantees an accepted change is not lost just because a
    /// worker was still pending when the last window closed.
    pub(crate) fn flush_on_exit(&self) {
        let Some(settings) = self.queue.latest() else {
            return;
        };
        if let Some(path) = &self.path {
            if let Err(error) = chaus_io::write_settings_rc(path, settings) {
                eprintln!("chaus: could not save preferences on exit: {error}");
            }
        }
    }
}

/// Install the writer once so both Preferences and the exit path share it.
pub(crate) fn install_writer(cx: &mut App, path: Option<PathBuf>) {
    if cx.try_global::<SettingsWriterGlobal>().is_none() {
        let writer = cx.new(|_| SettingsWriter::new(path));
        cx.set_global(SettingsWriterGlobal(writer));
    }
}
