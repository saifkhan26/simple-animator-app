# Walkthroughs: following one action through the code

Each walkthrough traces one thing a user does, step by step, with links to the function that does each step. Read
[architecture.md](architecture.md) first for the overall map. Line numbers are as of the commit named in each linked
page's header.

- [1. Launching the app](#1-launching-the-app)
- [2. Drawing one pencil line](#2-drawing-one-pencil-line)
- [3. Pressing Ctrl+Z](#3-pressing-ctrlz)
- [4. Saving the project](#4-saving-the-project)
- [5. Opening a project](#5-opening-a-project)
- [6. Pressing Play](#6-pressing-play)
- [7. A bucket fill](#7-a-bucket-fill)
- [8. Exporting an MP4](#8-exporting-an-mp4)
- [9. Edit in Krita, round trip](#9-edit-in-krita-round-trip)

---

## 1. Launching the app

1. The OS runs `main` → logging starts → `eframe::run_native` opens the window
   ([main.md](src/main.md#main)).
2. eframe calls the creation closure → [`AppState::new`](src/app.md#appstatenew): theme and fonts, GPU texture limit,
   `UiPrefs` from `ui.ron`, then [`with_prefs`](src/app.md#appstatewith_prefs) builds a 1280×720 24 fps
   [`Project::new`](src/doc/project.md#projectnew) (one layer keyed to one blank cell) and loads
   [`shortcuts::load`](src/input/shortcuts.md#load).
3. The custom canvas pipeline is built ([`CanvasGpu::new`](src/ui/canvas_gpu.md#canvasgpunew)).
4. First [`update`](src/app.md#appstateupdate): the window is maximised, rounded corners are applied
   ([platform.md](src/platform.md)), the pen tablet binds to the window, and
   [`ui::shell::draw`](src/ui/shell.md#draw) lays out the panels for the first time.

## 2. Drawing one pencil line

**Pen down.**
1. The canvas in [`ui::shell::draw`](src/ui/shell.md#draw) sees the press; pen packets from
   [`PenInput::poll`](src/input/tablet.md#peninputpoll) (or the mouse) are mapped to the drawing's pixel space by
   [`pen_stroke_points`](src/ui/shell.md#pen_stroke_points).
2. [`AppState::pointer_down`](src/app.md#appstatepointer_down): finds or creates the drawing to paint into
   ([`Project::ensure_active_cell`](src/doc/project.md#projectensure_active_cell); with auto-key on, a held frame
   gets its own key), takes a **snapshot** of its pixels (`stroke_pre_pixels`, often prepared already while the pen
   rested — `warm_pre`), and starts a [`StrokeBuilder`](src/tools/stroke.md) with the Pencil's brush and smoothing.

**Pen moves** (many samples per frame).
3. [`AppState::pointer_move`](src/app.md#appstatepointer_move) → [`StrokeBuilder::push`](src/tools/stroke.md#strokebuilderpush):
   smoothing, then a cubic Bezier is fitted through the samples and walked at the brush spacing, emitting **dabs**
   (pencil) or **spine nodes** (ink). Painting trails the pen by one sample.
4. Once the frame's samples are in, [`flush_stroke`](src/app.md#appstateflush_stroke) →
   [`StrokeBuilder::flush`](src/tools/stroke.md#strokebuilderflush): new dabs are rasterised into the coverage buffer
   ([ribbon.md](src/tools/ribbon.md)), shaped by grain and noise, clipped to the selection, and composited **over the
   snapshot** into the drawing. The changed rectangle is returned.
5. [`stream_stroke`](src/app.md#appstatestream_stroke) uploads that rectangle as a patch of the drawing's GPU texture
   *this frame*; [`live_tail`](src/ui/shell.md#live_tail) draws the not-yet-painted bit up to the pen, so the line
   reaches the nib with no visible lag.

**Pen up.**
6. [`AppState::pointer_up`](src/app.md#appstatepointer_up) → [`StrokeBuilder::finish`](src/tools/stroke.md#strokebuilderfinish)
   paints the last segment (and re-cuts flat ends).
7. [`commit_undo`](src/app.md#appstatecommit_undo) compares the snapshot with the drawing inside the stroke's dirty
   rectangle and pushes one `Command::PixelPatch` ([undo.md](src/undo.md)).
8. Next frame [`sync_textures`](src/app.md#appstatesync_textures) settles the texture (re-cropping it to the
   drawing if it grew).

With a **Krita brush** steps 3–4 instead go through [`KritaStroke`](src/tools/krita/mod.md): per spacing step,
sensors decide each dab's size, angle, opacity and scatter; the tip mask is built (or reused from the cache),
textured and composited by Krita's 8-bit rules.

## 3. Pressing Ctrl+Z

1. [`handle_keys`](src/app.md#appstatehandle_keys) → [`ShortcutMap::poll_actions`](src/input/shortcuts.md#shortcutmappoll_actions)
   reports `Action::Undo` → [`dispatch`](src/app.md#appstatedispatch) → [`AppState::undo`](src/app.md#appstateundo).
2. `undo` first backs out of anything in hand: a polygon corner, a lasso-fill step, a warp stroke, floating pixels
   ([`cancel_float`](src/app/select.md#appstatecancel_float)). If one applied, it stops there.
3. Otherwise [`History::undo`](src/undo.md#historyundo) pops the newest `Command` and [`apply`](src/undo.md#apply)
   runs it backwards — e.g. copies a `PixelPatch`'s `before` bytes back into the drawing and marks exactly that
   rectangle dirty.
4. [`apply_touched`](src/app.md#appstateapply_touched) refreshes what the undo changed (dirty textures, the selection,
   a grid); the next `sync_textures` uploads the restored pixels.

## 4. Saving the project

1. Ctrl+S → `dispatch(Action::SaveProject)` → [`save_project`](src/app.md#appstatesave_project): no file yet →
   [`save_project_as`](src/app.md#appstatesave_project_as) asks for a path
   ([`ask_save_path`](src/io/project_file.md)).
2. [`write_project`](src/app.md#appstatewrite_project) clones the project (cheap: drawings are `Arc`-shared) and spawns
   a worker thread running [`project_file::save_to`](src/io/project_file.md): header `ANIM` + version, then the project
   serialised with **postcard** through a **zlib** compressor, written to disk.
3. You keep drawing meanwhile; a drawing you edit is copied on its first write, so the save sees the old pixels.
4. Each frame [`poll_save_job`](src/app.md#appstatepoll_save_job) checks the channel: success → remember the path,
   show "Saved"; failure → an error dialog (the path stays as it was so a retry targets the same file).
5. Closing the app or opening another project first waits for the save
   ([`finish_pending_save`](src/app.md#appstatefinish_pending_save)).

## 5. Opening a project

1. `dispatch(Action::OpenProject)` → [`project_file::load_dialog`](src/io/project_file.md): pick a file, read the header,
   choose the reader for its version (older versions via [legacy.md](src/io/legacy.md)), stream-decompress and decode.
2. [`load_project`](src/app.md#appstateload_project) installs it: waits for any save, ends the Krita link, drops unused
   cells (`compact_cells`), restores the file's perspective grids, retires all textures, clears history, view, playback
   and in-progress edits. Every cell is marked for upload; the next frames build textures as drawings come into view.

## 6. Pressing Play

1. Space (default) → `dispatch(Action::PlayPause)` → [`play_pause`](src/app.md#appstateplay_pause) →
   [`Playback::toggle_unclocked`](src/timeline/playback.md#playbacktoggle_unclocked) (a shortcut has no clock reading;
   the first tick takes its time).
2. Every frame [`advance_playback`](src/app.md#appstateadvance_playback) → [`Playback::tick`](src/timeline/playback.md#playbacktick):
   whole frames elapsed since the current one started, at the project fps, move the playhead, wrapping around the loop
   range (or stopping at its end without looping).
3. `update` asks egui to repaint after `1/fps` seconds, so frames keep coming while playing.
4. The texture budget switches to "evict newest" during playback ([`evictions`](src/app.md#evictions)), so a loop that
   doesn't fit in memory still keeps most of it resident.
5. Pressing Play again stops and returns to the frame playback started on
   ([`return_to_play_start`](src/app.md#appstatereturn_to_play_start)).

## 7. A bucket fill

1. A Fill press → [`pointer_down`](src/app.md#appstatepointer_down) → a [`FillSession`](src/tools/fill.md#fillsessionnew) is
   built at the clicked pixel, reading walls from the drawing itself, from a linked line layer
   ([`fill_boundary`](src/app.md#appstatefill_boundary)) or from every visible layer
   ([`visible_lines`](src/app.md#appstatevisible_lines)).
2. [`FillSession::apply`](src/tools/fill.md#fillsessionapply): a scanline flood within tolerance
   ([`span_flood`](src/tools/fill.md#span_flood)); with a gap setting, a distance field
   ([`wall_distance`](src/tools/fill.md#wall_distance)) and a watershed ([`gap_flood`](src/tools/fill.md#gap_flood)) keep
   the fill from leaking through small breaks; on bare canvas it reaches into the lines' soft edges
   ([`soft_edges`](src/tools/fill.md#soft_edges)); then it's painted with the colour, opacity and grain.
3. Dragging before release re-applies with a different gap or expand (each apply first undoes the previous one).
4. Release → [`finish_fill`](src/app.md#appstatefinish_fill) records one undo step.

## 8. Exporting an MP4

1. File → Export → MP4 opens the export dialog; Export → [`start_export`](src/app.md#appstatestart_export) asks for a
   path, works out the frame order ([frame_order.md](src/io/frame_order.md) — the loop range, ping-pong, a camera move
   stretched over several loops), clones the project and spawns a worker thread.
2. On the worker, [`mp4_export::export_to`](src/io/mp4_export.md#export_to) starts `ffmpeg` reading raw RGB from its
   standard input; each frame is flattened through the camera ([io/composite.md](src/io/composite.md)), composited over
   black and piped in.
3. The UI shows a busy overlay; [`poll_bg_jobs`](src/app.md#appstatepoll_bg_jobs) collects the result and shows a toast
   or the error.

## 9. Edit in Krita, round trip

1. File → Edit in Krita → [`edit_in_krita`](src/app.md#appstateedit_in_krita): finds Krita, writes the project as an
   animated `.kra` on a worker ([`start_send`](src/app.md#appstatestart_send), [io/kra/mod.md](src/io/kra/mod.md)) and
   launches Krita on it.
2. You edit in Krita and save. Back in Animator, Edit in Krita again starts a **round**
   ([`drive_round`](src/app.md#appstatedrive_round)): the app writes `request` to the mailbox
   ([`tell`](src/krita_helper.md#tell)); the plugin ([extension.py](krita-plugin/animator_link/extension.md)) saves
   Krita's unsaved edits and answers `ready`.
3. The app reads Krita's save ([krita_link.md](src/krita_link.md)): only layers and drawings Krita actually changed are
   merged, as one undo step ([`apply_krita_pull`](src/app.md#appstateapply_krita_pull)).
4. The app rewrites the `.kra` (carrying Krita-only `-x` layers through untouched) and writes `sent`; the plugin reloads
   the document in place, keeping frame, active layer, zoom and rotation, and answers `reloaded`.
