//! The application-lifetime file-dialog service.
//!
//! Silvestris location ids are opaque handles owned by one provider instance:
//! `resolve()` only works for the provider that interned the id. Chaus therefore
//! constructs one `StdFsProvider` at startup and keeps it for the whole
//! application session, beside the separate Open and Save panel state and the
//! Open window's frame store. None of those values belong in the rc file; the
//! application supplies preference-backed stores.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, Bounds, Entity, Global, Pixels, WeakEntity};
use silvestris::file_panel::{
    FilePanel, FileProvider, MemoryPanelStateStore, PanelStateStore, StdFsProvider,
};
use silvestris::window_role::{FrameStore, SavedFrame, SavedFrames};

/// A frame store for tests and for a session whose preferences file is
/// unavailable. Silvestris only supplies the preference-backed adapter.
#[derive(Default)]
pub struct MemoryFrameStore {
    frames: RefCell<SavedFrames>,
}

impl MemoryFrameStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl FrameStore for MemoryFrameStore {
    fn load(&self, key: &str, _: &App) -> Option<Bounds<Pixels>> {
        self.frames.borrow().get(key).copied().map(Bounds::from)
    }

    fn save(&self, key: &str, bounds: Bounds<Pixels>, _: &mut App) {
        self.frames
            .borrow_mut()
            .insert(key.to_string(), SavedFrame::from(bounds));
    }
}

/// One shared provider, separate Open and Save presentation state, the Open
/// frame store and the most recently presented panel.
#[derive(Clone)]
pub struct FileDialogs {
    provider: Rc<StdFsProvider>,
    open_state: Rc<dyn PanelStateStore>,
    save_state: Rc<dyn PanelStateStore>,
    open_frames: Rc<dyn FrameStore>,
    last_panel: Rc<RefCell<Option<WeakEntity<FilePanel>>>>,
}

impl Global for FileDialogs {}

impl FileDialogs {
    #[must_use]
    pub fn new(
        provider: Rc<StdFsProvider>,
        open_state: Rc<dyn PanelStateStore>,
        save_state: Rc<dyn PanelStateStore>,
        open_frames: Rc<dyn FrameStore>,
    ) -> Self {
        Self {
            provider,
            open_state,
            save_state,
            open_frames,
            last_panel: Rc::new(RefCell::new(None)),
        }
    }

    /// A session-local service used by tests and by a window created without
    /// the application's preference-backed installation.
    #[must_use]
    pub fn ephemeral() -> Self {
        Self::new(
            Rc::new(StdFsProvider::new()),
            Rc::new(MemoryPanelStateStore::new()),
            Rc::new(MemoryPanelStateStore::new()),
            Rc::new(MemoryFrameStore::new()),
        )
    }

    /// Install the application service once. A later call leaves the running
    /// provider in place, because replacing it would invalidate every location
    /// handle the panels already own.
    pub fn install(cx: &mut App, dialogs: FileDialogs) {
        if cx.try_global::<Self>().is_none() {
            cx.set_global(dialogs);
        }
    }

    /// The service for this application session, creating an ephemeral one if
    /// the application has not installed its preference-backed service.
    pub fn ensure(cx: &mut App) -> Self {
        if cx.try_global::<Self>().is_none() {
            cx.set_global(Self::ephemeral());
        }
        cx.global::<Self>().clone()
    }

    #[must_use]
    pub fn provider(&self) -> Rc<dyn FileProvider> {
        self.provider.clone()
    }

    /// The provider's home directory, used when a document has no directory.
    #[must_use]
    pub fn home_path(&self) -> Option<PathBuf> {
        self.provider
            .home()
            .and_then(|location| self.provider.resolve(&location))
    }

    /// A directory for a panel that has no document context: home, or the
    /// launch directory on a host without a home.
    #[must_use]
    pub fn start_path(&self) -> Option<PathBuf> {
        self.home_path().or_else(|| std::env::current_dir().ok())
    }

    #[must_use]
    pub fn open_state(&self) -> Rc<dyn PanelStateStore> {
        self.open_state.clone()
    }

    #[must_use]
    pub fn save_state(&self) -> Rc<dyn PanelStateStore> {
        self.save_state.clone()
    }

    #[must_use]
    pub fn open_frames(&self) -> Rc<dyn FrameStore> {
        self.open_frames.clone()
    }

    /// Remember the panel just presented so the application (and its
    /// integration tests) can reach the real Silvestris panel in the window.
    pub fn remember_panel(&self, panel: &Entity<FilePanel>) {
        *self.last_panel.borrow_mut() = Some(panel.downgrade());
    }

    #[must_use]
    pub fn last_panel(&self) -> Option<Entity<FilePanel>> {
        self.last_panel
            .borrow()
            .as_ref()
            .and_then(WeakEntity::upgrade)
    }
}
