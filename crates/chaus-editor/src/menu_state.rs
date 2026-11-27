//! Titlebar menu ownership for one editor window.
//!
//! `MenuState` owns the per-window command registry, the lazily created popup
//! entity and a right-click request awaiting the next render. It does not own
//! command registration, checked-state publication or the menu model: those
//! stay on `Editor` and the command adapter.

use gpui::{Entity, Pixels, Point};

use crate::editor_commands::MenuKind;
use silvestris::command::CommandRegistry;
use silvestris::menu::PopupMenu;

/// Registry, popup entity and pending right-click request.
pub(crate) struct MenuState {
    commands: Entity<CommandRegistry>,
    popup: Option<Entity<PopupMenu>>,
    pending: Option<(MenuKind, Point<Pixels>)>,
}

impl MenuState {
    pub(crate) fn new(commands: Entity<CommandRegistry>) -> Self {
        Self {
            commands,
            popup: None,
            pending: None,
        }
    }

    pub(crate) fn commands(&self) -> &Entity<CommandRegistry> {
        &self.commands
    }

    pub(crate) fn has_popup(&self) -> bool {
        self.popup.is_some()
    }

    pub(crate) fn set_popup(&mut self, popup: Entity<PopupMenu>) {
        self.popup = Some(popup);
    }

    pub(crate) fn popup(&self) -> Option<Entity<PopupMenu>> {
        self.popup.clone()
    }

    pub(crate) fn queue_request(&mut self, kind: MenuKind, position: Point<Pixels>) {
        self.pending = Some((kind, position));
    }

    pub(crate) fn take_request(&mut self) -> Option<(MenuKind, Point<Pixels>)> {
        self.pending.take()
    }
}
