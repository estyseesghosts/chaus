//! Editor actions and their key bindings.

use gpui::{actions, App, KeyBinding};

/// Key context for editor-scoped bindings.
pub const KEY_CONTEXT: &str = "ChausEditor";

actions!(
    chaus_editor,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        Home,
        End,
        SelectHome,
        SelectEnd,
        DocumentStart,
        DocumentEnd,
        SelectDocumentStart,
        SelectDocumentEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        InsertNewline,
        InsertTab,
        Outdent,
        TrimTrailingWhitespace,
        ConvertIndentationToSpaces,
        ConvertIndentationToTabs,
        FormatDocument,
        Save,
        SaveAs,
        Find,
        Replace,
        FindNext,
        FindPrevious,
        ToggleVim,
        Escape,
        ZoomIn,
        ZoomOut,
        ZoomReset,
    ]
);

/// Bind editor keys. Primary shortcuts are Cmd on macOS and Ctrl elsewhere.
pub fn bind_keys(cx: &mut App) {
    macro_rules! primary {
        ($key:literal) => {
            if cfg!(target_os = "macos") {
                concat!("cmd-", $key)
            } else {
                concat!("ctrl-", $key)
            }
        };
    }
    let context = Some(KEY_CONTEXT);
    let mut bindings = vec![
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("shift-home", SelectHome, context),
        KeyBinding::new("shift-end", SelectEnd, context),
        KeyBinding::new("enter", InsertNewline, context),
        KeyBinding::new("tab", InsertTab, context),
        KeyBinding::new("shift-tab", Outdent, context),
        KeyBinding::new("escape", Escape, context),
        KeyBinding::new(primary!("a"), SelectAll, context),
        KeyBinding::new(primary!("c"), Copy, context),
        KeyBinding::new(primary!("x"), Cut, context),
        KeyBinding::new(primary!("v"), Paste, context),
        KeyBinding::new(primary!("z"), Undo, context),
        KeyBinding::new(primary!("shift-z"), Redo, context),
        KeyBinding::new(primary!("y"), Redo, context),
        KeyBinding::new(primary!("s"), Save, context),
        KeyBinding::new(primary!("shift-s"), SaveAs, context),
        KeyBinding::new(primary!("f"), Find, context),
        KeyBinding::new(primary!("h"), Replace, context),
        KeyBinding::new("f3", FindNext, context),
        KeyBinding::new("shift-f3", FindPrevious, context),
        KeyBinding::new(primary!("backspace"), DeleteWordLeft, context),
        KeyBinding::new(primary!("delete"), DeleteWordRight, context),
        KeyBinding::new(primary!("left"), WordLeft, context),
        KeyBinding::new(primary!("right"), WordRight, context),
        KeyBinding::new(primary!("shift-left"), SelectWordLeft, context),
        KeyBinding::new(primary!("shift-right"), SelectWordRight, context),
        KeyBinding::new(primary!("home"), DocumentStart, context),
        KeyBinding::new(primary!("end"), DocumentEnd, context),
        KeyBinding::new(primary!("shift-home"), SelectDocumentStart, context),
        KeyBinding::new(primary!("shift-end"), SelectDocumentEnd, context),
        KeyBinding::new("ctrl-shift-v", ToggleVim, context),
        KeyBinding::new(primary!("="), ZoomIn, context),
        KeyBinding::new(primary!("shift-="), ZoomIn, context),
        KeyBinding::new(primary!("-"), ZoomOut, context),
        KeyBinding::new(primary!("0"), ZoomReset, context),
        KeyBinding::new("ctrl-shift-t", TrimTrailingWhitespace, context),
        KeyBinding::new("ctrl-alt-s", ConvertIndentationToSpaces, context),
        KeyBinding::new("ctrl-alt-t", ConvertIndentationToTabs, context),
        KeyBinding::new(primary!("shift-i"), FormatDocument, context),
    ];
    if !cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("shift-delete", Cut, context),
            KeyBinding::new("ctrl-insert", Copy, context),
            KeyBinding::new("shift-insert", Paste, context),
        ]);
    }
    cx.bind_keys(bindings);
}
