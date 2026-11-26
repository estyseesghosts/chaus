//! Editor actions and their key bindings.

use gpui::{actions, App, KeyBinding};

/// Key context for editor-scoped bindings.
pub const KEY_CONTEXT: &str = "ChausEditor";
/// Key context while Vim owns text keys (Normal, Visual and Command modes).
/// Editing bindings are excluded so Enter, Backspace, arrows and Delete reach
/// the modal engine instead of inserting or deleting text.
pub const VIM_OWNED_KEY_CONTEXT: &str = "ChausEditor ChausVimNormal";
/// Key context while a modal sheet owns the window. GPUI dispatches key
/// bindings before element key handlers, so `ChausEditor` must disappear from
/// the context stack or Indent/Newline would still edit the document behind
/// the sheet.
pub const MODAL_KEY_CONTEXT: &str = "ChausModal";
/// Editor bindings never fire while a Silvestris text client owns the keyboard.
const COMMAND_CONTEXT: &str = "ChausEditor && !SilvestrisText";
/// Context predicate for bindings that only apply outside Vim's text-owning
/// modes, including Insert mode.
const EDIT_CONTEXT: &str = "ChausEditor && !ChausVimNormal && !SilvestrisText";

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
    let context = Some(COMMAND_CONTEXT);
    let edit = Some(EDIT_CONTEXT);
    let bindings = vec![
        KeyBinding::new("backspace", Backspace, edit),
        KeyBinding::new("delete", Delete, edit),
        KeyBinding::new("left", Left, edit),
        KeyBinding::new("right", Right, edit),
        KeyBinding::new("up", Up, edit),
        KeyBinding::new("down", Down, edit),
        KeyBinding::new("shift-left", SelectLeft, edit),
        KeyBinding::new("shift-right", SelectRight, edit),
        KeyBinding::new("shift-up", SelectUp, edit),
        KeyBinding::new("shift-down", SelectDown, edit),
        KeyBinding::new("home", Home, edit),
        KeyBinding::new("end", End, edit),
        KeyBinding::new("shift-home", SelectHome, edit),
        KeyBinding::new("shift-end", SelectEnd, edit),
        KeyBinding::new("enter", InsertNewline, edit),
        KeyBinding::new("tab", InsertTab, edit),
        KeyBinding::new("shift-tab", Outdent, edit),
        KeyBinding::new(primary!("backspace"), DeleteWordLeft, edit),
        KeyBinding::new(primary!("delete"), DeleteWordRight, edit),
        KeyBinding::new(primary!("left"), WordLeft, edit),
        KeyBinding::new(primary!("right"), WordRight, edit),
        KeyBinding::new(primary!("shift-left"), SelectWordLeft, edit),
        KeyBinding::new(primary!("shift-right"), SelectWordRight, edit),
        KeyBinding::new(primary!("home"), DocumentStart, edit),
        KeyBinding::new(primary!("end"), DocumentEnd, edit),
        KeyBinding::new(primary!("shift-home"), SelectDocumentStart, edit),
        KeyBinding::new(primary!("shift-end"), SelectDocumentEnd, edit),
        KeyBinding::new("escape", Escape, context),
        KeyBinding::new(primary!("a"), SelectAll, context),
        KeyBinding::new("f3", FindNext, context),
        KeyBinding::new("shift-f3", FindPrevious, context),
        KeyBinding::new(primary!("="), ZoomIn, context),
        KeyBinding::new(primary!("shift-="), ZoomIn, context),
        KeyBinding::new(primary!("-"), ZoomOut, context),
        KeyBinding::new(primary!("0"), ZoomReset, context),
        KeyBinding::new("ctrl-shift-t", TrimTrailingWhitespace, context),
        KeyBinding::new("ctrl-alt-s", ConvertIndentationToSpaces, context),
        KeyBinding::new("ctrl-alt-t", ConvertIndentationToTabs, context),
    ];
    // Undo, redo, cut, copy, paste, find, replace, format, save, save-as,
    // close, preferences, New, Open and the Vim toggle are command bindings
    // generated from the per-window `CommandRegistry`; binding them here again
    // would make one chord dispatch twice.
    cx.bind_keys(bindings);
}
