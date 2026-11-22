//! Menu commands, checked state and the window's menu bar.
//!
//! Every menu item routes through Silvestris's `CommandRegistry` and invokes
//! the same editor methods as the keyboard actions. The registry is per window,
//! so the checked Vim/Wrap items and disabled Undo/Redo follow the focused
//! document instead of a global stale value.

use chaus_core::settings::WrapMode;
use gpui::{point, px, App, AppContext as _, Context, Entity, WeakEntity, Window};
use silvestris::command::CommandRegistry;
use silvestris::controls::{Button, ButtonStyle};
use silvestris::core::command::{CommandDescriptor, CommandId, MenuEntry, MenuModel};
use silvestris::menu::PopupMenu;

use crate::editor::Editor;
use crate::editor_keys::{Copy, Cut, Paste, Redo, ToggleVim, Undo};

/// View → Wrap radio items, in menu order.
const WRAP_COMMANDS: [(&str, &str, WrapMode); 3] = [
    ("view.wrap_word", "Word", WrapMode::Word),
    ("view.wrap_char", "Character", WrapMode::Character),
    ("view.wrap_none", "No Wrap", WrapMode::None),
];

fn register_command<F>(
    commands: &Entity<CommandRegistry>,
    cx: &mut Context<Editor>,
    descriptor: CommandDescriptor,
    handler: F,
) where
    F: Fn(&CommandId, &mut Window, &mut App) + 'static,
{
    commands
        .update(cx, |registry, cx| {
            registry.register(descriptor, handler, cx)
        })
        .expect("editor command ids are unique");
}

/// Menu shown by a top-level button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuKind {
    File,
    Edit,
    View,
}

impl MenuKind {
    pub(crate) const ALL: [Self; 3] = [Self::File, Self::Edit, Self::View];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
        }
    }

    pub(crate) fn model(self) -> MenuModel {
        let entries = match self {
            Self::File => vec![
                MenuEntry::Command("file.new".into()),
                MenuEntry::Command("file.open".into()),
                MenuEntry::Separator,
                MenuEntry::Command("file.save".into()),
                MenuEntry::Command("file.save_as".into()),
                MenuEntry::Separator,
                MenuEntry::Command("file.close".into()),
                MenuEntry::Separator,
                MenuEntry::Command("settings.open".into()),
            ],
            Self::Edit => vec![
                MenuEntry::Command("edit.undo".into()),
                MenuEntry::Command("edit.redo".into()),
                MenuEntry::Separator,
                MenuEntry::Command("edit.cut".into()),
                MenuEntry::Command("edit.copy".into()),
                MenuEntry::Command("edit.paste".into()),
                MenuEntry::Separator,
                MenuEntry::Command("edit.find".into()),
                MenuEntry::Command("edit.replace".into()),
                MenuEntry::Separator,
                MenuEntry::Command("edit.format".into()),
            ],
            Self::View => vec![
                MenuEntry::Command("view.vim_mode".into()),
                MenuEntry::Separator,
                MenuEntry::Submenu {
                    label: "Wrap".to_string(),
                    entries: vec![
                        MenuEntry::Command("view.wrap_word".into()),
                        MenuEntry::Command("view.wrap_char".into()),
                        MenuEntry::Command("view.wrap_none".into()),
                    ],
                },
            ],
        };
        MenuModel {
            label: self.label().into(),
            entries,
        }
    }
}

impl Editor {
    pub(crate) fn create_command_registry(cx: &mut Context<Self>) -> Entity<CommandRegistry> {
        cx.new(|_| CommandRegistry::default())
    }

    /// Register the commands this editor owns. App-owned commands (New, Open,
    /// Preferences) are registered by `chaus-app` on the same registry.
    pub(crate) fn register_editor_commands(&self, weak: WeakEntity<Self>, cx: &mut Context<Self>) {
        let commands = self.commands.clone();

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("file.save", "Save").shortcut("secondary-s"),
            move |_, _, cx| {
                editor
                    .update(cx, |editor, cx| {
                        editor.save_inner(false, cx);
                    })
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("file.save_as", "Save As").shortcut("secondary-shift-s"),
            move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| {
                        editor.save_as_inner(false, window, cx);
                    })
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("file.close", "Close Window").shortcut("secondary-w"),
            move |_, window, cx| {
                let allow = editor
                    .update(cx, |editor, cx| editor.confirm_close(window, cx))
                    .unwrap_or(false);
                if allow {
                    window.remove_window();
                }
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("edit.undo", "Undo").shortcut("secondary-z"),
            move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| editor.undo(&Undo, window, cx))
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("edit.redo", "Redo").shortcut("secondary-shift-z"),
            move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| editor.redo(&Redo, window, cx))
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("edit.cut", "Cut").shortcut("secondary-x"),
            move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| editor.cut(&Cut, window, cx))
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("edit.copy", "Copy").shortcut("secondary-c"),
            move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| editor.copy(&Copy, window, cx))
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("edit.paste", "Paste").shortcut("secondary-v"),
            move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| editor.paste(&Paste, window, cx))
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("edit.find", "Find").shortcut("secondary-f"),
            move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| editor.open_find(false, window, cx))
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("edit.replace", "Replace").shortcut("secondary-h"),
            move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| editor.open_find(true, window, cx))
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::action("edit.format", "Format Document")
                .shortcut("secondary-shift-i"),
            move |_, _, cx| {
                editor
                    .update(cx, |editor, cx| editor.format_document(cx))
                    .ok();
            },
        );

        let editor = weak.clone();
        register_command(
            &commands,
            cx,
            CommandDescriptor::toggle("view.vim_mode", "Vim Mode", false).shortcut("ctrl-shift-v"),
            move |_, window, cx| {
                editor
                    .update(cx, |editor, cx| {
                        editor.toggle_vim(&ToggleVim, window, cx);
                    })
                    .ok();
            },
        );

        for (id, label, mode) in WRAP_COMMANDS {
            let editor = weak.clone();
            register_command(
                &commands,
                cx,
                CommandDescriptor::radio(id, label, "wrap", mode == WrapMode::Word),
                move |_, _, cx| {
                    editor
                        .update(cx, |editor, cx| editor.set_wrap_mode(mode, cx))
                        .ok();
                },
            );
        }
    }

    /// Create the popup menu on first render, when a window is available.
    pub(crate) fn ensure_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            return;
        }
        let model = MenuKind::File.model();
        let registry = self.commands.clone();
        let menu = cx.new(|cx| PopupMenu::new(model, registry, window, cx));
        self.menu = Some(menu);
    }

    pub(crate) fn open_menu(
        &mut self,
        kind: MenuKind,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_menu(window, cx);
        let Some(menu) = self.menu.clone() else {
            return;
        };
        menu.update(cx, |menu, cx| {
            menu.set_model(kind.model(), window, cx);
            menu.open(position + point(px(0.), px(12.)), window, cx);
        });
    }

    /// Publish per-window state so menu items are checked/disabled correctly.
    pub(crate) fn sync_command_state(&mut self, cx: &mut Context<Self>) {
        let commands = self.commands.clone();
        let vim = self.vim;
        let wrap = self.settings.wrap;
        let can_undo = self.document.can_undo();
        let can_redo = self.document.can_redo();
        let has_selection = !self.selection.is_empty();
        let can_save = self.file.is_none() || self.document.is_dirty();
        let can_format = self.file.as_ref().is_some_and(|file| {
            chaus_io::format::Formatter::for_path(&file.meta.user_requested_path).is_some()
        });
        commands.update(cx, |registry, cx| {
            let _ = registry.publish(
                CommandDescriptor::toggle("view.vim_mode", "Vim Mode", vim)
                    .shortcut("ctrl-shift-v"),
                cx,
            );
            for (id, label, mode) in WRAP_COMMANDS {
                let _ = registry.publish(
                    CommandDescriptor::radio(id, label, "wrap", wrap == mode),
                    cx,
                );
            }
            let _ = registry.publish(
                CommandDescriptor::action("edit.undo", "Undo")
                    .shortcut("secondary-z")
                    .enabled(can_undo),
                cx,
            );
            let _ = registry.publish(
                CommandDescriptor::action("edit.redo", "Redo")
                    .shortcut("secondary-shift-z")
                    .enabled(can_redo),
                cx,
            );
            for (id, label, shortcut) in [
                ("edit.cut", "Cut", "secondary-x"),
                ("edit.copy", "Copy", "secondary-c"),
            ] {
                let _ = registry.publish(
                    CommandDescriptor::action(id, label)
                        .shortcut(shortcut)
                        .enabled(has_selection),
                    cx,
                );
            }
            let _ = registry.publish(
                CommandDescriptor::action("file.save", "Save")
                    .shortcut("secondary-s")
                    .enabled(can_save),
                cx,
            );
            let _ = registry.publish(
                CommandDescriptor::action("edit.format", "Format Document")
                    .shortcut("secondary-shift-i")
                    .enabled(can_format),
                cx,
            );
        });
    }

    pub(crate) fn render_menu_bar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        use gpui::{div, IntoElement as _, ParentElement as _, Styled as _};
        let mut bar = div().flex().items_center().gap(px(2.0));
        for kind in MenuKind::ALL {
            bar = bar.child(
                Button::new(
                    format!("menu-{}", kind.label().to_ascii_lowercase()),
                    kind.label(),
                )
                .style(ButtonStyle::Textured)
                .on_click(cx.listener(
                    move |editor, event: &gpui::ClickEvent, window, cx| {
                        editor.open_menu(kind, event.position(), window, cx);
                    },
                )),
            );
        }
        bar.into_any_element()
    }
}
