Current heads are **Chaus `1aedd17f`** and **Silvestris `devel/sil-cocoa-fixture` `30173a58`**. The important change from the previous plan is that the provider-hardening work is now substantially done upstream: exact opaque path handles, non-UTF-8 preservation, fallible existence checks, provider-owned filename validation, capability-driven menus, platform Trash, async Places, symlink handling, and `SaveDisposition::{New, ReplaceConfirmed}` are all present and tested. [Silvestris changelog](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/CHANGELOG.md#L14-L55) [real `StdFsProvider` tests](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/tests/std_fs_provider.rs)

So I would **stop redesigning the panels now and put them into Chaus**. Any generic failures Chaus exposes should go back upstream to Silvestris.

1. **P0 — Re-pin Chaus to `30173a58` and enable the real filesystem provider.**  
   **Deliverables:** change the workspace Silvestris revision from `a62b3b25` to `30173a5860ad263feacb3d417a73c3a354a21e78`, enable `features = ["std-fs"]`, regenerate the lockfile, and get the untouched Chaus tree compiling/tests passing before changing behavior. Keep the existing GPUI `6b720973...` pin; it already matches Silvestris. Chaus is still on the old Silvestris revision today. [Chaus workspace dependency](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/Cargo.toml#L24-L35) Current Silvestris documents `StdFsProvider` as the production host-filesystem adapter rather than a gallery fixture. [file-panels.md](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/docs/file-panels.md#L140-L197)

   **Avoidables:** no GPUI upgrade in the same change, no path dependency to a sibling Silvestris checkout, no replacement of Chaus's existing platform application wrapper, and no panel integration until this baseline passes.

   **Regression traps:** accidentally resolving two GPUI revisions; forgetting `std-fs` on one Chaus crate that directly references `StdFsProvider`; Windows/Linux target dependencies exposed by the newly enabled provider.

2. **P0 — Add one application-lifetime Chaus file-dialog service.**  
   **Deliverables:** create a small Chaus-owned application service/global containing one shared `Rc<StdFsProvider>`, Open panel state, Save panel state, and the Open-panel frame store. The **same provider instance must live for the entire app session**, because current Silvestris intentionally makes `LocationId`s opaque handles owned by that provider. `home()`, `root()` and `location_for()` intern real paths; `resolve()` converts them back to exact `PathBuf`s. [current `StdFsProvider`](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/src/file_panel/std_fs/mod.rs#L1-L105)

   Use separate Open and Save `PanelState` values; do not let changing Save's expanded state overwrite Open's view state. For the finished application, persist these UI values through a small Chaus UI-preferences document using `PreferencePanelStates` and `PreferenceFrames`; keep them out of `.chausrc`. Silvestris explicitly treats panel state and frame persistence as consumer-owned state. [panel state API](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/src/file_panel/state.rs#L1-L110) [frame-store API](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/src/window_role.rs#L259-L350)

   **Avoidables:** do not construct a new provider for every Open/Save invocation; do not persist its opaque `fs-location-*` identifiers; persist actual user choices/state and reconstruct locations from host paths on startup.

   **Regression traps:** stale provider IDs after accidentally replacing the provider; Open and Save sharing one `PanelState`; corrupt UI-state data preventing Chaus from starting. UI-state persistence should fail soft.

3. **P0 — Move Chaus's file/application shortcuts onto the Silvestris command path before replacing Open.**  
   **Deliverables:** install `CommandScope` around the editor window and use the editor's existing `CommandRegistry` as the authoritative path for application/document commands. Generate bindings from the registry for commands such as New, Open, Save, Save As, Close, Preferences, Undo/Redo, Cut/Copy/Paste, Find/Replace, Format and Vim toggle. Keep actual editor-language input—arrows, Backspace, Insert-mode Tab/Enter, Vim Normal keys, selections—inside Chaus's editor bindings. Silvestris's command bindings already automatically exclude focused `SilvestrisText` controls. [Silvestris command dispatch](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/src/command.rs#L30-L245) [command contract](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/docs/commands.md#L1-L70)

   This matters for Open specifically: Chaus currently registers `file.open` in its registry, but its handler just dispatches the app-global `OpenFile` action; that action lacks a `Window` and therefore uses GPUI's native `prompt_for_paths`. [Chaus `register_app_commands`](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-app/src/app.rs#L280-L327) [current native Open action](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-app/src/app.rs#L190-L235) The Silvestris registry callback already receives `&mut Window`, which is exactly what `OpenPanel::run_modal` needs.

   **Avoidables:** do not install registry shortcuts on top of Chaus's existing duplicate Ctrl+S/Ctrl+O/etc. bindings. Migrate a chord to one owner and remove the old binding in the same change.

   **Regression traps:** Vim Normal mode losing ownership of editor keys; Ctrl+Shift+V firing while a panel TextField owns focus; shortcuts invoking a disabled command because keyboard and menu routes diverged.

4. **P0 — Replace File → Open with the actual Silvestris `OpenPanel`.**  
   **Deliverables:** make `file.open` create:

   ```rust
   OpenPanel::new(file_dialogs.provider.clone())
       .directory_path(start_directory)
       .title("Chaus: Open")
       .allows_multiple(false)
       .can_choose_files(true)
       .can_choose_directories(false)
       .state_store(...)
       .frame_store(...)
       .on_finish(...)
       .run_modal(window, cx);
   ```

   `directory_path()` now exists specifically for host-path consumers, and `run_modal()` is the intended File → Open presentation: a real application-modal, resizable `WindowRole::OpenPanel`, not a sheet. [current OpenPanel API](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/src/file_panel/open.rs#L20-L145)

   Start from the current document's parent where sensible; otherwise use the provider's home directory. Chaus edits arbitrary plaintext, so initially **do not set `allowed_types`**. On an accepted outcome, take the exact returned `PathBuf` and feed it into Chaus's existing background `chaus_io::load` path. Preserve Chaus's current File → Open behavior of opening the selection in its own document window rather than replacing an unrelated dirty document. [current asynchronous Open load](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-app/src/app.rs#L196-L235)

   **Avoidables:** `:e path`, command-line paths and reload operations already have explicit destinations and must remain direct Chaus file operations; do not make every open operation summon a panel. Do not perform decoding inside the `on_finish` callback.

   **Regression traps:** Cancel/Escape/close-window must open nothing; a failed file load must report an error without leaving a fake successfully-opened document; modal Open must prevent the underlying editor from consuming keystrokes; Unicode and non-UTF-8 host paths must reach `chaus_io` intact.

5. **P0 — Replace Save As and untitled Save with the real `SavePanel`, without replacing Chaus's save engine.**  
   **Deliverables:** preserve `FileRequest`, `DocumentGeneration`, `FileOperation::ChoosingSaveTarget`, `CloseAfter`, `SaveTarget`, the save gate and Chaus's existing asynchronous write/conflict logic. Replace only `cx.prompt_for_new_path()` inside `save_as_inner()` with `SavePanel::begin_sheet`. [current Chaus picker boundary](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-editor/src/editor_file.rs#L229-L338) [file-operation ownership](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-editor/src/file_operation.rs#L1-L105)

   Configure the panel with the current file's parent/name when one exists, otherwise home + `"Untitled"`. Use `ExtensionPolicy::Any`: Chaus must not silently append `.txt` or reject a source file because its extension is unfamiliar. [SavePanel API](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/src/file_panel/save.rs#L20-L170)

   The crucial new integration is `SaveDisposition`. Current Silvestris now tells Chaus whether the chosen target was new or whether the user explicitly approved replacement. [outcome contract](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/src/file_panel/panel.rs#L44-L76) Map it directly into Chaus:

   ```text
   SaveDisposition::New
       -> SaveTarget.refuse_existing = true

   SaveDisposition::ReplaceConfirmed
       -> SaveTarget.refuse_existing = false
   ```

   That is important. If another process creates a target after SavePanel checked it as new, Chaus's atomic save refuses to clobber it. If the user actually clicked Replace, overwrite semantics are intentional. Current Silvestris's Save flow uses fallible `exists()`, keeps IO errors in the panel, and reports `ReplaceConfirmed` only after its replacement alert resolves. [current Save policy](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/src/file_panel/panel.rs#L1075-L1195)

   **Avoidables:** do not let the panel write the file; Silvestris explicitly ends at target selection. Do not bypass `chaus_io::save`, atomic replacement, `DurabilityUnconfirmed`, recovery or Chaus's document-generation guards. Explicit `:w path` / `:saveas path` must continue bypassing the visual panel.

   **Regression traps:** a second Save As while the sheet is active; editing after the panel accepts but before the write finishes; owner-window closure while the Save sheet is open; SavePanel cancel during `CloseAfter::Sheet`; Replace alert → Cancel returning to the Save sheet; an old callback arriving after the editor has changed documents.

6. **P0 — Make Chaus's save-close flow cooperate cleanly with the new sheet lifecycle.**  
   **Deliverables:** when a dirty-close prompt chooses Save on an untitled document, dismiss/hide that prompt before starting `SavePanel`, carry the same `CloseAfter::Sheet` state through target selection and writing, and re-present the close prompt when SavePanel is cancelled or the write fails. Chaus already encodes exactly this policy in `CloseAfter` and `close_not_started()`. [Chaus `CloseAfter`](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-editor/src/file_operation.rs#L35-L68) [cancel/restore behavior](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-editor/src/editor_file.rs#L340-L350)

   This is also the first meaningful real-world test of Silvestris's one-sheet-per-window rule and delayed result delivery after sheet retraction. SavePanel now uses the actual `sheet::Sheet` machinery; it should not need a Chaus-specific overlay hack. [Silvestris panel presentation contract](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/docs/file-panels.md#L45-L67)

   **Avoidables:** no second Save sheet over the same document; no stacking the replace-existing alert as another document sheet—Silvestris correctly opens that collision alert application-modally.

   **Regression traps:** Cancel should return to the original dirty-close decision instead of silently abandoning the close request; successful Save should close exactly once; a save conflict should leave the document open and dirty.

7. **P1 — Adopt current Silvestris window semantics while Chaus is being converted.**  
   **Deliverables:** make document frames explicitly use `WindowRole::Document`; make Preferences use `WindowRole::Preferences`; use `WindowFrame::edited(document.is_dirty())`; remove `" — Edited"` from `Editor::title()` so dirty state is represented through the Aqua/Geist window chrome rather than duplicated in the title string. Chaus currently still generates titles such as `"foo.txt — Edited"`. [current Chaus title](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-editor/src/editor.rs#L388-L405) Current Silvestris explicitly defines document/preferences/Open-panel roles. [window roles](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/src/window_role.rs#L18-L145)

   Remove Chaus's redundant **left-button** transparent titlebar drag/zoom handler once the current `WindowFrame` handles that area correctly. Preserve the special right-click menu hook separately if Chaus still wants it.

   **Avoidables:** do not let `WindowRole::Document` silently change Chaus's current 420×300 minimum if that remains the practical editor minimum.

   **Regression traps:** correct title after Save As/open/recovery; dirty dot after edits made during an in-flight save; Windows/Linux drag/maximize behavior; titlebar right-click must not steal traffic-light or resize hit targets.

8. **P1 — Finish the four-theme and editor-font contract instead of creating another theme system.**  
   **Deliverables:** retain Chaus's existing four values—Aqua Light, Aqua Dark, Geist Light, Geist Dark—and make them a tested product contract. Chaus already maps all four to Silvestris `ThemeId`s. [Chaus theme mapping](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-app/src/theme_choice.rs) Silvestris itself explicitly models family and appearance independently. [Silvestris theme IDs](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris-core/src/theme.rs#L5-L58)

   Preserve the editor-font model Chaus already has:

   ```text
   settings.font_family = Some(...)
       -> use user's editor font

   settings.font_family = None
       -> use active Silvestris theme.type_.mono
   ```

   Chaus already does exactly that when shaping editor rows. [editor font selection](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-editor/src/editor_paint.rs#L37-L75)

   Current Silvestris gives Aqua **Source Code Pro** and Geist **Geist Mono**, so the requested defaults fall out naturally. [Aqua typography](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris-core/src/aqua.rs#L39-L52) [Geist typography](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris-core/src/geist.rs#L41-L54)

   Fix Preferences so its placeholder is dynamic—currently it incorrectly always says `"Theme default (Geist Mono)"`. Under Aqua it should identify Source Code Pro; under Geist it should identify Geist Mono. [current Preferences font field](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-app/src/preferences.rs#L64-L83) Add a clear/reset-to-theme-default action and validate whether an explicitly requested family exists in GPUI's text system; if unavailable, surface a warning and render with the theme default rather than accepting an unpredictable platform fallback.

   Chaus's existing Geist Sans override should remain a **UI-font override only**. It should not alter the editor's mono face. [Chaus Geist UI font](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-app/src/fonts.rs) Existing tests already establish Geist Sans + Geist Mono vs Aqua Lato + Source Code Pro. [font tests](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-app/tests/font_overrides.rs)

   **Avoidables:** do not set the user's editor font through Silvestris's global mono override; that would mutate generic framework monospace controls too. Do not store `"Source Code Pro"` or `"Geist Mono"` as explicit user settings when the user has selected “theme default”—`None` is what allows the default to follow family changes.

   **Regression traps:** Aqua→Geist must change the implicit editor font; Light→Dark within one family must not; an explicit custom font must survive all four theme switches; font/theme changes must invalidate shaping/wrapping without resetting document, selection, undo or scroll state.

9. **P1 — Replace picker mocks with real Chaus↔Silvestris integration tests.**  
   **Deliverables:** rewrite the Chaus tests that currently assert `did_prompt_for_new_path()` and call `simulate_new_path_selection()`. Those tests explicitly exercise GPUI's old native-prompt path. [current Save tests](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-editor/tests/menus.rs#L218-L324) [file-operation picker test](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/crates/chaus-editor/tests/file_operation.rs#L55-L108)

   New tests should open a real Chaus editor window against a temporary directory, invoke Chaus's actual File commands, interact with the real Silvestris panel selectors, and verify the filesystem and editor afterwards. Cover Open accept/cancel, Save-new, Save-replace, Replace→Cancel, SavePanel cancel, second Save As rejection, close→Save→cancel restoration, edits while write is in flight, a new target appearing between panel acceptance and write, new folder/rename, list/icon/column navigation, and panel state reuse.

   Do not duplicate every Silvestris unit test in Chaus. Silvestris already directly tests opaque paths, real rename/folder creation, fallible existence and symlinks against a real temporary filesystem. [current provider tests](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/crates/silvestris/tests/std_fs_provider.rs) Chaus's responsibility is to prove the **consumer boundary**: panel outcome → Chaus load/save state machine → actual document/file.

   **Avoidables:** no fake Chaus wrapper around `MemoryProvider` as the primary integration test. MemoryProvider remains appropriate for Silvestris component tests, not proof that Chaus works with real paths.

   **Regression traps:** reduced-motion timing versus real sheet animations; application-modal Open owning a separate test window; tests passing on Aqua Light only; Windows and Unix path behavior diverging unnoticed.

10. **P1 — Run the complete UI/theme/platform acceptance matrix through Chaus and feed generic defects back into Silvestris.**  
   **Deliverables:** manually dogfood Open and Save on Windows and Linux across Aqua Light, Aqua Dark, Geist Light and Geist Dark. Exercise all three browser views, large directories, hidden files, inaccessible directories, Unicode names, Unix non-UTF-8 names where possible, symlinked directories, New Folder → inline rename, Move to Trash, Replace, keyboard-only navigation, panel resizing and reopened panel state. Test common fractional DPI scales as well as 100%/200%.

   If Chaus reveals a generic problem—panel focus, `StdFsProvider`, sheet animation, view switching, filename validation, Trash, theme rendering—fix it in **Silvestris with a Silvestris regression test**, then repin Chaus. Do not add a Chaus-only workaround. Silvestris's consumer documentation explicitly lists Chaus's Open/Save gap as #29 and notes Chaus has no local Silvestris patches. [consumer gap register](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/docs/consumers.md#L382-L440)

   **Avoidables:** do not narrow `StdFsProvider` around Chaus-specific assumptions. Chaus owns editor/file policy; Silvestris owns reusable panel/filesystem presentation behavior.

   **Regression traps:** fixing a panel bug only for `StdFsProvider` when it is actually generic `FilePanel` behavior; fixing a Chaus save-policy bug inside Silvestris; allowing Aqua fixes to regress Geist or vice versa.

11. **P2 — Delete the obsolete picker routes and mark Chaus as the reference consumer.**  
   **Deliverables:** once the integration tests are green, remove `PathPromptOptions`, `cx.prompt_for_paths`, `cx.prompt_for_new_path`, the old picker-specific test seams, and `chaus-dialog` if a final code search confirms it has no remaining independent purpose. It is still a workspace member today. [Chaus workspace](https://github.com/estyseesghosts/chaus/blob/1aedd17f703133b19e9dc42c4baa272388ebc5af/Cargo.toml#L1-L20)

   Update Silvestris `docs/consumers.md` to Chaus's new pin and replace the current `chaus | Open/Save | #29` gap with an adoption note pointing at the real Chaus integration. That makes Chaus the regression vehicle for future OpenPanel/SavePanel changes instead of merely another consumer waiting for them. [current consumer record](https://github.com/estyseesghosts/silvestris/blob/30173a5860ad263feacb3d417a73c3a354a21e78/docs/consumers.md#L382-L440)

   **Avoidables:** do not retain a silent native-picker fallback. If Silvestris Open/Save breaks, Chaus should expose that failure; otherwise it is not actually serving as the testbed.

   **Regression traps:** leaving stale `chaus-dialog` dependencies or tests behind; accidentally routing explicit Ex commands through GUI panels; documentation claiming Chaus uses #29 while production still has an alternate picker path.

The critical path is **pin → shared provider/state → command routing → OpenPanel → SavePanel → integration tests**. Window-role cleanup and the four-theme/font work can proceed alongside the later panel testing, but I would not remove the old picker implementation until the real Silvestris-panel tests are exercising actual Chaus IO successfully.
