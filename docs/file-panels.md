# File panels

Chaus uses Silvestris's real `OpenPanel` and `SavePanel`; there is no native
picker route left in the application.

## Ownership

`chaus-editor::FileDialogs` is the application-lifetime service:

- one `Rc<StdFsProvider>` for the whole session. `LocationId`s are opaque
  handles owned by that instance, so the provider is never rebuilt for a
  dialog;
- a separate `PanelStateStore` for Open and Save, so changing Save's expanded
  state cannot overwrite Open's view;
- the Open window's `FrameStore`;
- a weak handle to the last panel for diagnostics and tests.

`chaus-app` installs it from `ui_preferences::install_file_dialogs`, backed by
`_chaus-ui.json` / `.chaus-ui.json`. A missing file yields defaults; a malformed
one is reported and never overwritten. Panel state and frames are not written to
`.chausrc`.

## Open

`file.open` creates `OpenPanel::run_modal` in a `WindowRole::OpenPanel` window
titled "Chaus: Open". It starts at the current document's parent (absolutized
from a relative CLI path) or at the provider's home/launch directory. Only an
accepted outcome with a host `PathBuf` reaches `chaus_io::load`; the load runs on
the background executor and opens a new document window. Cancel, Escape and
closing the Open window open nothing. A load failure creates no fake document;
the existing error path opens an Untitled window with a truthful status.

`chaus` edits arbitrary plaintext, so Open sets no `allowed_types`.

## Save As

`save_as_inner` keeps `FileRequest`, `DocumentGeneration`,
`FileOperation::ChoosingSaveTarget`, `CloseAfter`, `SaveTarget`, the save gate
and the asynchronous write/conflict logic. Only the picker changes: it now calls
`SavePanel::begin_sheet` with `ExtensionPolicy::Any`. The sheet starts at the
current file's parent/name, or home + `Untitled`.

The panel returns a target; it never writes. `SaveDisposition` maps to the
existing engine:

```text
New                -> SaveTarget.refuse_existing = true
ReplaceConfirmed   -> SaveTarget.refuse_existing = false
```

A target that appears after the panel checked it as new becomes a conflict, not
a clobber. Replace means the user approved replacement. Explicit `:w path`,
`:saveas path`, `:e path` and `:r path` continue to bypass the sheet.

Dirty-close `Save` dismisses the close sheet, carries `CloseAfter::Sheet`
through selection and writing, and restores the close prompt if the panel is
cancelled or the write fails. A successful save closes exactly once.

## Automated coverage

`crates/chaus-editor/tests/file_panel_save.rs`:

- Save on a named document writes without a sheet;
- Save As new target, then association and title;
- Replace alert then Replace;
- Replace alert Cancel returns to the sheet, then sheet Cancel leaves the file
  and dirty state alone;
- sheet Cancel;
- dirty close → Save → sheet Cancel restores the close prompt;
- a successful close-save writes once and removes exactly its own window;
- an edit made while the write is in flight stays dirty (the test schedules the
  write behind an asynchronous timer seam, so the deterministic scheduler can
  edit after the snapshot and before the commit);
- a target created between acceptance and write is not clobbered (same seam);
- New Folder on the real Save sheet creates a directory and Chaus writes inside
  it;
- all three browser views and last-view reuse.

`crates/chaus-app/tests/file_panels.rs`:

- File → Open Cancel opens no document;
- accepting a Unicode path (`nöte-文件.txt`) through the real panel opens that
  exact name in a new Chaus window.

`crates/chaus-app/tests/theme_contract.rs` covers the four theme ids, implicit
mono per family, explicit-family survival and the Geist UI-only override.

`crates/silvestris/tests/std_fs_provider.rs` (upstream) exercises real rename
(including case-only and collision refusal), New Folder, Move to Trash,
non-UTF-8 names and symlink classification against a temporary filesystem.
Those are provider operations that do not cross Chaus's document state machine,
so Chaus does not duplicate them; Chaus tests the boundary where a panel outcome
becomes a loaded or written document.

## Manual acceptance still required

Headless tests prove the panel-outcome → load/save state machine and GPUI
dispatch, not native window management. Before release, run the interactive
`.github/workflows/native-gui.yml` gate on self-hosted Windows and X11 runners;
`scripts/smoke/windows.ps1` and `scripts/smoke/linux.sh` cover panel accept,
Escape cancel, filename-only Save As, Replace and Replace-cancel, Unicode
names, focus restoration and dirty-close routing with real keyboard input.
Before release, also run this manual matrix on real Windows and Linux desktops:

- File → Open and Save As in Aqua Light, Aqua Dark, Geist Light, Geist Dark.
- List, Icon and Column views; large directories; hidden files; inaccessible
  directories; Unicode names; Unix non-UTF-8 names; symlinked directories.
- New Folder → inline rename; Move to Trash.
- Keyboard-only navigation, panel resizing, reopened panel state.
- 100%, 125%, 150%, 200% and common fractional DPI scales.
- Windows drag/maximize and titlebar right-click against traffic lights and
  resize edges; Linux client-side decorations.

WSLg is not a substitute for the Linux desktop rows: it applies its own scale
factor, so a WSLg launch is a smoke test only, not Linux DPI acceptance. The
automated suites and release builds do run on both Windows and Linux.

A generic defect found here belongs in Silvestris with a Silvestris regression
test, then a Chaus re-pin. A Chaus policy defect (save gating, dirty-close
restoration, document association) stays in Chaus.
