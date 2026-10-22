# VEDIT

**A minimalist, timeline-based video editor written in pure Rust.**

VEDIT is a lightweight alternative to heavy NLEs like Adobe Premiere: a clean,
dark, distraction-free interface with a multi-track timeline, media library,
live preview and a full-featured export pipeline — powered under the hood by
ffmpeg, so it opens, edits and renders *every* format your machine can throw
at it.

```
┌──────────────────────────────────────────────────────────────────┐
│ VEDIT 0.3.0   New  Open…  Save  Import  Undo  Redo  Mixer [Export]│
├──────────────┬───────────────────────────────────────┬───────────┤
│  MEDIA       │                                       │ INSPECTOR │
│  ┌────────┐  │           ▶  P R E V I E W            │  Clip /   │
│  │ thumb  │  │                                       │  Title /  │
│  └────────┘  │  |<  <<   Play   >>  >|   00:04:12:07 │  Grade /  │
│  intro.png   ├───────────────────────────────────────┴───────────┤
│  music.wav   │ + Video + Audio + Title  Snap  Split  Undo  Zoom  │
│              │ TIMECODE ──────────────────────────────────────── │
│              │ V2 │ Title ╭─xx─╮ ┌──image──┐                     │
│              │ V1 │ ▂▃▅ clip 1 ▅▃▂╼╾╼ clip 2 ╾╼──────── ▶          │
│              │ A1 │ ▁▂▄~ music.wav ~▄▂▁ (waveform)                 │
│              │ A2 │                                                │
└──────────────┴───────────────────────────────────────────────────┘
```

## Features

- **Workspace tabs (Edit · Color · Audio)** — one click (or Alt+1/2/3)
  switches the entire workplace, Venturi/DaVinci-style:
  * **Edit** — media pool, preview, inspector, timeline.
  * **Color** — preview + docked scopes rack + full grade page (four wheels,
    scalars, presets, LUT).
  * **Audio** — a full mixer console page with tall faders and meters.
- **Minimalist UI** — dark charcoal + single amber accent, generous spacing,
  zero clutter, native egui rendering, instant startup.
- **Multi-track timeline** — unlimited video & audio tracks, drag to move,
  drag edges to trim, split at playhead, magnetic snapping, zoomable ruler,
  per-track mute / solo / lock / delete in the header column.
- **Linked A/V clips** — dropping a video that carries sound creates a
  Premiere-style linked audio twin on the audio track; moving, trimming,
  splitting and deleting stay in sync for both halves.
- **Waveform & thumbnail strips** — audio clips draw their real waveform,
  video/image clips show a filmstrip; both are cached on disk and mapped
  through trims and speed.
- **Transitions & fades** — per-clip fade in/out (video alpha + audio
  volume) and cross-dissolves between adjacent clips on a track; markers
  are drawn right on the timeline.
- **Undo / redo** — full history (Ctrl+Z / Ctrl+Y), gesture-coalesced,
  100 steps deep, with a **jumpable history dropdown** in the top bar.
- **Ripple delete** — remove a clip and close the gap in one step
  (Shift+Del or the toolbar button).
- **Keyframes** — opacity & volume keyframes per clip with linear
  interpolation; previewed live and rendered exactly on export.
- **Text & title clips** — styled titles (embedded DejaVu fonts, bold,
  color, alignment, position) rendered identically in preview and export.
- **Audio mixer** — real console strips: vertical dB faders (−60…+12 with
  scale ticks), balance pan knobs, SOLO/MUTE, live peak meters with
  hold, one-click Normalize (loudest clip → −1 dBFS) and a master strip —
  either floating in Edit or full-page in the Audio workspace.
- **6-band parametric EQ** — interactive response curve per track (drag
  points: frequency/gain, wheel: Q, double-click: on/off) with presets
  (Voice clarity, De-esser, Broadcast loud, Music mastering…). Runs LIVE
  in the preview (Windows/macOS) and maps onto native filters on export.
- **3-band multiband compressor** — threshold/ratio/attack/release/gain per
  band + crossovers, presets, and live gain-reduction metering; live in
  the preview, exported with `acompressor` per band.
- **Inspector with tabs** — Video/Audio tabs like the reference app:
  Transform (scale X/Y, position, rotation, pivot, flip), Crop, Blend
  modes (Add/Multiply/Screen/Overlay/Soft-light), opacity — all with
  keyframe diamonds.
- **Transform & blend in preview AND export** — scale/rotate/position/crop
  are composited per-frame in the preview and translated to
  `crop/rotate/scale/overlay/blend` filters on export (keyframed
  transforms become per-frame expressions).
- **Copy / paste attributes** — take one clip's whole look (transform, crop,
  blend, grade, fades, keyframes, volume) with Ctrl+C and apply it to any
  other clip with Ctrl+V.
- **Color grading** — four Resolve-style luma wheels (Shadows / Midtones /
  Highlights / Offset with color dot + Luma + Sat), band ranges, presets
  (Neutral, Black & White, gray-world Auto Balance), exposure / contrast /
  saturation / temperature / tint and `.cube` 3D LUT support — in the
  Inspector AND in the dedicated Color workspace.
- **Scopes, integrated with color** — a docked two-slot scope rack in the
  Color workspace: Waveform, RGB Parade, Vectorscope (with 75 % bar targets
  and skin-tone line) and Histogram, measured from the composed preview
  frame; slot choices persist between sessions. Also available as a
  floating window in the Edit workspace.
- **Hardware-accelerated export** — NVENC (NVIDIA), Quick Sync (Intel) and
  AMF (AMD) H.264/HEVC encoders with availability detection, next to the
  full software codec set.
- **Settings & autosave** — persisted preferences (autosave interval,
  snapping, default preview quality, ffmpeg folder override) in
  `~/.vedit/settings.json`; projects autosave in the background.
- **Media library** — import video / audio / images, thumbnails, searchable,
  drag assets straight onto the timeline (or drag files from Explorer into
  the window).
- **Live preview** — frame-accurate compositing of all visible layers
  (transforms, crop, blend modes, grades), quality selector
  (Full / Half / Quarter), playback with audio*.
- **Optimized media (proxies)** — 4K/8K sources automatically get a 1080p
  proxy generated in the background at import; the preview, thumbnails and
  waveforms use the proxy while exports always read the ORIGINAL file.
  This is what makes 4K editing smooth on ordinary machines.
- **Clip properties** — position, duration, speed (0.25×–4×), opacity,
  volume, per-clip & per-track mute.
- **Projects** — everything (assets + timeline + effects + mixer) saved to a
  single portable `.vedit.json` file.
- **Keyboard-driven** — Space, S, Del, Shift+Del (ripple), Ctrl+C/V
  (attributes), Alt+1/2/3 (workspaces), arrows, Home/End,
  Ctrl+Z/Y/S/E/O/I.

\* Live audio preview (Windows & macOS) decodes ANY format through ffmpeg
into a 48 kHz stereo mix bus — AAC/MP4 (`.mov`/`.mp4`/`.m4a`), MP3, WAV,
FLAC, OGG/Vorbis, AC3, whatever your ffmpeg supports — and runs the full
mixer (faders, pan, EQ, compressor, master) in real time on the audio
thread. Linux builds play video-only preview (exports always include audio
on every platform).

## Formats & codecs — the ffmpeg advantage

VEDIT's engine shells out to **ffmpeg/ffprobe**, which means:

| | supported |
|---|---|
| **Import** | MP4, MKV, MOV, AVI, WebM, FLV, WMV, MPEG, TS, 3GP, OGV, MXF… · MP3, WAV, AAC, FLAC, OGG, M4A, WMA, Opus, AIFF, AC3… · PNG, JPG, GIF, BMP, WebP, TIFF, TGA, AVIF, EXR… — *anything ffmpeg demuxes* |
| **Video codecs (export)** | H.264 (libx264), H.265/HEVC (libx265), VP9, AV1 (SVT-AV1), Apple ProRes 422, VP8, MPEG-4, FFV1 (lossless), GIF · **hardware:** H.264/HEVC via NVENC, Quick Sync and AMF |
| **Audio codecs (export)** | AAC, MP3, Opus, Vorbis, FLAC, PCM/WAV |
| **Containers (export)** | MP4, MKV, MOV, WebM, AVI, MPG, TS, OGV, GIF, WAV, MP3, FLAC, OGG, M4A |
| **Controls** | bitrate **or** CRF quality, encoder speed preset, resolution, frame rate, sample rate, custom ffmpeg args |

Every VEDIT export is a real ffmpeg render with a full filter graph:
per-track layering with opacity curves, fades, cross-dissolves, title
overlays, color grades + `.cube` LUTs, source trimming, speed changes
(`setpts` / `atempo`), keyframed volume ramps, track pan/EQ, `amix` mixing
with a master fader and palette-optimised GIFs.

## Project layout

```
VEDIT/
├── Cargo.toml                  # cargo workspace
├── crates/
│   ├── vedit-core/             # engine: project model, ffprobe, export renderer,
│   │   │                       #         preview decoder, timeline ops, waveform,
│   │   │                       #         text rasterizer, LUT parser, color grade
│   │   └── src/{lib,project,timeline,media,probe,ffmpeg,export,preview,thumbnail,error,
│   │           waveform,textrender,lut,colorgrade}.rs
│   └── vedit-gui/              # egui app: panels, theme, audio preview, settings
│       └── src/{main,app,theme,audio,settings}.rs + src/panels/*
├── assets/                     # icon.png / icon.ico
├── installer/
│   ├── vedit.iss               # Inno Setup script  → VEDIT-Setup-x.y.z.exe
│   └── vedit.nsi               # NSIS alternative
├── scripts/
│   ├── build-windows.ps1       # one-command build + dist + installer
│   └── build.sh                # linux/macOS build
└── .github/workflows/release.yml  # tag → cloud-built .exe + installer
```

The GUI is 100 % Rust (egui/eframe). The heavy media lifting is delegated to
the ffmpeg binaries — the same approach used by many production tools — which
is what makes "every format, every codec" work on day one.

---

## Building & installing

### Requirements

- **Rust** (stable) — <https://rustup.rs>
- **ffmpeg + ffprobe** — put them on `PATH`, or next to the executable in an
  `ffmpeg/` folder, or set `VEDIT_FFMPEG_DIR`
- Windows builds need the MSVC C++ Build Tools ("Desktop development with C++")
- Linux dev deps: `libxkbcommon-dev libwayland-dev libx11-dev`

### Run from source (any OS)

```bash
cd VEDIT
cargo run --release -p vedit-gui
```

### Windows — build the .exe + installer yourself

Option A — **one command** (builds, downloads ffmpeg, compiles the installer):

```powershell
cd VEDIT
.\scripts\build-windows.ps1 -BuildInstaller
# → dist\VEDIT\vedit.exe            (portable app)
# → installer\Output\VEDIT-Setup-0.3.0.exe   (the installer)
```

The script auto-detects Inno Setup (`winget install JRSoftware.InnoSetup` if
missing). Without `-BuildInstaller` you still get the portable
`dist\VEDIT\` folder — `vedit.exe` + `ffmpeg\` — which runs anywhere.

Option B — **manual steps**:

```powershell
# 1. build
cargo build --release -p vedit-gui
# 2. get ffmpeg (essentials build) and place the two exes here:
#    dist\VEDIT\ffmpeg\ffmpeg.exe
#    dist\VEDIT\ffmpeg\ffprobe.exe
# 3. compile the installer
winget install JRSoftware.InnoSetup
& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" /DAppVersion=0.3.0 installer\vedit.iss
# → installer\Output\VEDIT-Setup-0.3.0.exe
```

The installer puts `vedit.exe` **and** `ffmpeg\ffmpeg.exe`/`ffprobe.exe` into
`C:\Program Files\VEDIT`, adds Start-Menu/Desktop shortcuts and a proper
uninstaller. VEDIT auto-finds ffmpeg next to its own exe — nothing to
configure.

Option C — **let GitHub build it for you (zero local setup)**:

1. Push this folder to a GitHub repository.
2. Tag a release: `git tag v0.3.0 && git push origin v0.3.0`.
3. The `release` workflow builds `vedit.exe`, bundles ffmpeg, compiles
   `VEDIT-Setup-0.3.0.exe` and a portable zip, and uploads them as artifacts
   on the Actions run page.

### Linux / macOS

```bash
./scripts/build.sh        # or: cargo build --release -p vedit-gui
./target/release/vedit
```

Uses the system ffmpeg (`sudo apt install ffmpeg` / `brew install ffmpeg`).

---

## Using VEDIT

1. **New Project** — pick resolution & frame rate (or open/import right away).
2. **Import Media…** (Ctrl+I) or drag files into the window.
3. Drag assets from the library onto a track (or press **＋ Add** to insert at
   the playhead).
4. Edit: drag clips to move (cross-track works), drag clip edges to trim,
   **S** splits at the playhead, **Del** removes, Inspector tunes
   speed/opacity/volume, adds fades, transitions, keyframes, titles and
   color grades. **Mixer** opens per-track faders, pan, EQ and solo.
5. **Space** plays; scrub by dragging the ruler or empty lane.
6. **Export** (Ctrl+E): pick a preset or tune codec (software or GPU)/
   bitrate/CRF/resolution/fps yourself, choose the output file, hit
   **Export** and watch the live progress bar (cancel any time).

## Keyboard shortcuts

| Key | Action |
|---|---|
| `Space` | Play / Pause |
| `S` | Split clip(s) at playhead |
| `Del` / `Backspace` | Delete selected clip |
| `Shift+Del` | **Ripple delete** — remove and close the gap |
| `Ctrl+C` / `Ctrl+V` | Copy / paste clip **attributes** (look + effects) |
| `Alt+1` / `Alt+2` / `Alt+3` | Switch workspace: Edit / Color / Audio |
| `Ctrl+Z` / `Ctrl+Y` | Undo / redo (`Ctrl+Shift+Z` also redoes) |
| `←` / `→` | Step 1 frame (`Shift` = 10 frames) |
| `Home` / `End` | Jump to start / end |
| `Ctrl+S` / `Ctrl+E` / `Ctrl+O` / `Ctrl+I` | Save / Export / Open / Import |
| `Wheel` / `Ctrl+Wheel` | Scroll timeline / zoom at cursor |

## Roadmap

- [x] Multi-track timeline with move/trim/split/snap
- [x] Full-ffmpeg export (bitrate/CRF, presets, GIF palette)
- [x] Frame-accurate composited preview
- [x] Linked A/V clips (audio twin auto-created for videos with sound)
- [x] Track headers with mute / lock / delete
- [x] Waveform & thumbnail strips on clips
- [x] Transitions (cross-dissolve) & fade in/out
- [x] Undo/redo history
- [x] Keyframed opacity/volume
- [x] Text & title clips
- [x] Hardware-accelerated encode (NVENC/QSV/AMF)
- [x] Audio mixer (faders/pan/EQ/compressor/meters/normalize, live in preview)
- [x] 6-band parametric EQ + 3-band multiband compressor with presets
- [x] Transform / crop / blend modes (preview + export, keyframeable)
- [x] Color grading: luma wheels + presets + auto balance + .cube LUT
- [x] Scopes: waveform / RGB parade / vectorscope / histogram in a docked
      Color-workspace rack (persisted slots)
- [x] Workspace tabs: Edit · Color · Audio (Alt+1/2/3)
- [x] Ripple delete (close gap)
- [x] Copy/paste clip attributes
- [x] Jumpable undo history
- [x] Optimized media (1080p proxies) for smooth 4K+ editing
- [x] Settings persistence + autosave
- [ ] More transition kinds (dip to black, wipes)
- [ ] Per-property keyframe graphs / easing curves
- [ ] Linux live audio preview

## License

MIT — see [LICENSE](LICENSE). ffmpeg is an external binary and is **not**
distributed with this repository; the build scripts download official builds
for you (check each build's licensing: LGPL/GPL).
