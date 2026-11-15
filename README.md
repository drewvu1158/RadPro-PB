# Radiate Photobooth

A local photo booth for Windows, written in Rust.

Turn a PC with a webcam into a photo booth: start a session, stand in front
of the camera, watch the countdown, get a strip of photos saved to disk —
like a booth at a wedding or party.

> **Independent implementation.** Original Rust code; no Sparkbooth application
> code or template artwork is bundled. Users can import their own compatible
> layouts. This app is not affiliated with Sparkbooth / John Wu Presents, LLC.
> "Sparkbooth" is a trademark of its owner.

## What it does

- A black interface with warm peach accents, Georgia headings, Segoe UI controls,
  live preview, and photo review. The app uses installed Windows fonts when available
  and falls back to its bundled fonts; no system font is redistributed. The capture
  screen includes a live indicator, a transparent countdown overlay, and paper-style photo cards.
- **Go fullscreen** enters guest mode with operator settings, studio controls,
  camera details and file paths hidden. The guest screen is a portrait kiosk:
  **Take picture** and **Exit fullscreen** at the top, the mirrored live preview
  in the middle, and the shots from this session along the bottom. F11 also
  toggles fullscreen, and Escape leaves OS fullscreen while keeping the current
  photo session.
- Live camera preview (webcam via Media Foundation, through `nokhwa`)
- **Take picture** starts one countdown (1–10 s, configurable). After each shot,
  the booth waits for another click; guests choose when to take every picture.
  Failed captures can be retried without discarding completed photos.
- Tap a captured photo in the filmstrip, then **Retake photo** to replace only
  that shot. The original stays intact until the replacement succeeds. During
  a retake countdown, **Keep original photo** cancels the replacement.
- On the review screen, drag a photo directly onto another photo on the full
  template to swap their positions. The highlighted frame shows the drop target;
  dropping on the border or outside the template leaves the order unchanged.
  Repeated slots for the same shot stay linked.
- You can also select a thumbnail and use **Move earlier / Move later**
  to arrange the print order, or **Retake photo** to try that shot again. The
  preview, saved layout, and print use the same order; edits save a new version.
- The bottom filmstrip has one slot per shot in the session — numbered
  placeholders until each capture lands, with the newest shot outlined in the
  accent color. Frames are uploaded to it as they arrive, so guests can judge a
  shot before taking the next one. The whole camera frame remains visible;
  saved photos keep the original camera orientation. Longer sessions wrap the
  filmstrip on tall, narrow screens; compact windows keep a single row.
- Photo review stacks the preview and print controls on portrait screens, with
  selectable thumbnails and a new-session button. Wide screens place controls
  beside the preview.
- 1–8 shots per session (default 4)
- Composes a classic vertical photo strip or a 2×2 grid with a border
- **Layout studio**: a three-pane designer with editable text, PNG/JPEG images,
  photo placements, and rectangle, ellipse, heart, star, and diamond shapes.
  Resize with corner handles, rotate with the round handle, or enter exact
  positions, sizes and angles. Includes a front-to-back layer list with
  visibility, locking, duplication, deletion and stacking controls.
- **Typography**: multiline text with wrapping, Sans/Serif/Mono typefaces, size,
  alignment, bold, italic, tracking, line height, color and opacity. Sans and
  Mono are bundled fonts; Serif uses the host's Georgia font when available
  and falls back to Sans. Complex-script shaping and custom font import are
  not implemented.
- **Artwork styling**: image contain/cover/stretch modes, flips, opacity,
  rounded corners and borders; editable shape fills and borders. The base
  background supports solid color, linear/radial gradients, dots and checks,
  with eight color palettes and custom color pickers.
- **Ten original premade templates**: Editorial, After Hours, Garden Party,
  Pop Club, Golden Hour, Made of Memories, Electric Night, Birthday Wishes,
  Modern Love, and Less Is More. Every element is editable. Blank strip,
  double-strip and 4×6 grid layouts are also available.
- **History and precision**: undo/redo (40 steps), canvas alignment, grid,
  zoom, clean preview, keyboard nudging and snapping. **Save .sbl** preserves
  editable layers; **Apply to booth** saves a separate active copy.
- **Alignment guides** mark matching edges and centers of photo boxes and the
  canvas while dragging, including the visible bounds of rotated layers. Layers
  snap within six screen points, regardless of zoom. Hold **Shift** to move
  freely, or turn off **Snap to guides**. Guides disappear on release.
- **Photo review**: browse the finished layout and each captured photo.
- **Print layout** opens the Windows printer/paper/copies dialog, and prints
  the finished layout only after confirmation. The whole layout fits inside
  the printable page; this does not configure dye-sub cutters or borderless
  printer settings. A submitted job is not proof of physical completion.
- **Send me my photos**: visible Email/Text section with disabled input and
  send controls. No addresses are collected, no requests are sent, and no
  AWS resources are created. This section is reserved for the future backend.
- Auto-saves the composed strip (JPEG) and optionally the individual shots
  to `<Pictures>/Sparkbooth-rust` (configurable)
- Settings screen (shots, countdown, layout, border, output folder, camera
  selection, simulated-camera mode), persisted as TOML
- Camera picker with a **Reconnect** button, so switching between a built-in
  webcam, a USB webcam, or a virtual camera (OBS, EOS Webcam Utility, …) does
  not need an app restart
- `--list-cameras` prints what the camera backend sees and whether each device
  actually delivers frames
- Simulated camera feed for machines without a webcam (`--simulate`, or the
  "Simulated camera" setting): an animated test pattern that exercises the
  whole pipeline including capture and strip composition

## Build & run

Requires a stable Rust toolchain and the MSVC C++ build tools.

```sh
cargo run            # launch the kiosk
cargo run -- --simulate   # launch with the simulated camera feed
cargo run -- --list-cameras   # show detected cameras and test each one
cargo run -- --simulate --settings C:/path/to/test-settings.toml
cargo test           # run the unit tests
cargo build --release
```

### Choosing the right camera

Windows exposes virtual cameras as ordinary cameras, so a machine can list
several devices even with one webcam attached. Open **Settings → Camera** and
pick the device by name; the preview reconnects immediately. `--list-cameras`
shows the index, the negotiated format, and whether frames actually arrive:

```
Media Foundation reports 2 camera(s):
  index 0: EOS Webcam Utility (Index(0))
  index 1: Logi C270 HD WebCam (Index(1))
```

If the preview stays black, the selected device is probably a virtual camera
whose helper app is not running. Pick the physical webcam instead.

Use the release build for everyday use — debug builds are several times
slower and much heavier on the CPU. The release binary is a single ~7 MB
`radiate-photobooth.exe` with no runtime to install.

## SBL compatibility

Imports the AMF3 **version 2** `.sbl` layout format: canvas dimensions,
`photoindex` (including duplicate strips), `orderindex`, top-left-origin
rotation, and embedded `backgroundimage`/`overlayimage` bytes. The captured
photo count follows the highest referenced shot number, rather than the
number of boxes. Original files are never modified by import or Apply.
Export writes the same v2 AMF3 structure and preserves unedited metadata.
Radiate designs store their editable layer stack in a versioned
`radiate_design` extension and embed flattened background/overlay PNGs with
the ordinary photo placements for other SBL readers. Arbitrary artwork/photo
interleaving is projected into those PNGs using photo coverage. Flattened
alpha compositing can differ by a few color levels due to 8-bit rounding;
reopening in Radiate retains the original editable composition. Photo frames
are opaque rectangles, matching the SBL photo model. Image and shape layers
can have rounded corners and transparency.
Opening exported files in the original Sparkbooth application has not been
verified. JSON layout files and other SBL versions are not supported.

Separate text/image layers encoded by Sparkbooth are still rejected with an
explanation; flatten those into a background/overlay in Sparkbooth before
exporting. Radiate-created text/image layers remain fully editable. Missing
external artwork is reported rather than fetched or silently omitted. Files
are limited to 32 MiB, canvas/artwork to 16 megapixels and 8192 pixels per side,
with 1–64 photo placements referencing shots 1–8, 128 total layers, and a
combined decoded image-layer budget of 32 megapixels. Text is limited to 4096
UTF-8 bytes per layer. These bound decoded memory and document complexity.

## Designing a layout

Open **Layout studio**, then **Templates / Presets** to choose a starter.
Use **+ Photo**, **+ Text**, **+ Image**, or **+ Shape** to add elements.
Select an element on the canvas or in **Layers**; the right inspector exposes
its content, geometry and styling. The layer at the top of the list is in
front. Lock a background or decoration to keep it out of canvas selection.

The **Canvas** tab controls dimensions, base background, patterns and palettes.
**Background image** adds an ordinary image layer behind the other elements.
Use the inspector to change any image's crop mode or replace its source.
Hide **Outlines** for a clean preview. All artwork is embedded on save.

Drag to move, use corner squares to resize, and use the round handle to
rotate. Hold **Shift** to bypass movement snapping, keep the original aspect
ratio while resizing, or snap rotation to 15-degree increments. Arrow keys
nudge one pixel; Shift+arrow nudges ten. Ctrl+D duplicates, Delete removes,
Ctrl+Z undoes, Ctrl+Y redoes, and Ctrl+S opens Save. Keyboard shortcuts leave
text fields' normal editing behavior intact.

Changing presets and importing files can be undone. Changes remain a draft
until **Apply to booth**. Save a `.sbl` to keep a design for later; drafts are
not autosaved on app exit. Overwritten files receive timestamped backups.

Format inspection and compatibility checks use two publicly shared templates
from [A1 Photo Booth's sharing library](https://www.a1photobooth.com/SB/), linked
by the [Sparkbooth website](https://sparkbooth.com/news/unofficial-sparkbooth-layout-library/):

- [softhearts.sbl](https://www.a1photobooth.com/SB/SB-Layouts/softhearts.sbl):
  1800×1200, four rotated placements, embedded PNG background.
  SHA-256 `9c83545266ffb7545747bca5cc75372969b52fa49bd54ac193d4eb9c26e4d765`.
- [Save-the-Date-1x3-Duplicate.sbl](https://www.a1photobooth.com/SB/SB-Layouts/Save-the-Date-1x3-Duplicate.sbl):
  1200×1800, six placements using three captures, embedded background/overlay.
  SHA-256 `00ef9329de4baca1d5dd5a5888d1006b8fc5d882531a4db65aeecfe2b1193ce1`.

These files are not redistributed. To run the optional compatibility check,
download them into a temporary directory as `softhearts.sbl` and
`duplicate.sbl`, then set `SPARKBOOTH_SBL_SAMPLE_DIR` to that directory and run:

```powershell
cargo test --release template::tests::public_sbl_samples_import_render_and_round_trip -- --ignored --nocapture
```

The check verifies geometry, artwork, rendering and import/export/render
round trips, writing preview PNGs beside the samples. It never prints.

## Easy on the hardware

The booth is built to sit in the background on modest hardware:

- The camera streams at 640×480 — exactly the strip cell size — so the booth
  never decodes a larger frame than it can use, and the preview is published
  at **15 fps**.
- A capture is a single extra grab from the same stream, taken **only when a
  shot is due**, never for the preview.
- The UI repaints only as often as the current screen needs (30 fps in the
  preview/countdown, 4 fps on the review screen) and still responds instantly
  to clicks and keys.
- Rendering uses the lightweight OpenGL (`glow`) backend rather than a full
  GPU abstraction layer, keeping startup fast and memory low.
- The simulated feed bakes its gradient background once per size; each frame
  is a cheap copy plus a moving ball, so testing without a webcam is light too.
- There is no background network activity, telemetry, or idle polling beyond
  the preview tick.

Earlier baseline on the development machine (before Layout studio; release build, simulated feed, 1280×800
window): about 87 MB working set and roughly 5% of one CPU core while the
preview animates. A real webcam adds decode work proportional to its frame
rate. Memory and CPU with imported template artwork have not been measured.

## Configuration

`%APPDATA%/sparkbooth/sparkbooth-rust/config/settings.toml` — created on first change in the
settings screen. Photos are written to `<Pictures>/Sparkbooth-rust` by
default.

The exact settings path is also shown in **Settings**, below the output folder.
The existing configuration and photo folder locations are retained from the
earlier app name, so saved camera choices, templates and output paths carry over.
Applied layouts live beside it as `active-layout.sbl`. Changed settings and
overwritten layout files receive timestamped backups. `--settings PATH` can
select an isolated configuration for testing or a separate booth setup.

## Out of scope (for now)

Deliberately not included in v0.1. Ideas for later:

- Native decoding of Sparkbooth's separate text/graphic layer encoding
- Sound effects and voice countdowns
- Direct dye-sub cutter control and automatic printer setup
- GIF/animated output, cloud upload/sharing
- On-screen keyboard, RFID/NFC start, multi-camera selection quality
- AI-generated photo backdrops

Contributions welcome as ordinary, original code.

## License

MIT
