# VEDIT

**A minimalist, timeline-based video editor written in pure Rust.**

VEDIT is a lightweight alternative to heavy NLEs like Adobe Premiere: a clean,
dark, distraction-free interface with a multi-track timeline, media library,
live preview and a full-featured export pipeline — powered under the hood by
ffmpeg, so it opens, edits and renders *every* format your machine can throw
at it.

```
┌──────────────────────────────────────────────────────────────────┐
│ VEDIT 0.1.0   New  Open…  Save  Import Media…          [Export]  │
├──────────────┬───────────────────────────────────────┬───────────┤
│  MEDIA       │                                       │ INSPECTOR │
│  ┌────────┐  │           ▶  P R E V I E W            │  Clip /   │
│  │ thumb  │  │                                       │  Project  │
│  └────────┘  │  |<  <<   Play   >>  >|   00:04:12:07 │  settings │
│  clip1.mp4   ├───────────────────────────────────────┴───────────┤
│  intro.png   │ + Video  + Audio  Snap  Split  Delete   Zoom ──●  │
│  music.wav   │ TIMECODE ──────────────────────────────────────── │
│              │ V2 │      ┌────────────┐                          │
│              │ V1 │ ═════╪ clip 1     ╞═══════════════ ▶          │
│              │ A1 │ ─────┰─ music.wav ┸─────────────              │
│              │ A2 │                                                  │
└──────────────┴───────────────────────────────────────────────────┘
```

## Features

- **Minimalist UI** — dark charcoal + single amber accent, zero clutter,
  native egui rendering, instant startup.
- **Multi-track timeline** — unlimited video & audio tracks, drag to move,
  drag edges to trim, split at playhead, magnetic snapping, zoomable ruler.
- **Media library** — import video / audio / images, thumbnails, drag assets
  straight onto the timeline (or drag files from Explorer into the window).
- **Live preview** — frame-accurate compositing of all visible layers,
  quality selector (Full / Half / Quarter), playback with audio*.
- **Clip properties** — position, duration, speed (0.25×–4×), opacity,
  volume, per-clip & per-track mute.
- **Projects** — everything (assets + timeline + settings) saved to a single
  portable `.vedit.json` file.
- **Keyboard-driven** — Space, S, Del, arrows, Home/End, Ctrl+S/E/O/I.

\* Live audio preview is enabled on Windows & macOS builds; Linux builds play
video-only preview (exports always include audio on every platform).

## Formats & codecs — the ffmpeg advantage

VEDIT's engine shells out to **ffmpeg/ffprobe**, which means:

| | supported |
|---|---|
| **Import** | MP4, MKV, MOV, AVI, WebM, FLV, WMV, MPEG, TS, 3GP, OGV, MXF… · MP3, WAV, AAC, FLAC, OGG, M4A, WMA, Opus, AIFF, AC3… · PNG, JPG, GIF, BMP, WebP, TIFF, TGA, AVIF, EXR… — *anything ffmpeg demuxes* |
| **Video codecs (export)** | H.264 (libx264), H.265/HEVC (libx265), VP9, AV1 (SVT-AV1), Apple ProRes 422, VP8, MPEG-4, FFV1 (lossless), GIF |
| **Audio codecs (export)** | AAC, MP3, Opus, Vorbis, FLAC, PCM/WAV |
| **Containers (export)** | MP4, MKV, MOV, WebM, AVI, MPG, TS, OGV, GIF, WAV, MP3, FLAC, OGG, M4A |
| **Controls** | bitrate **or** CRF quality, encoder speed preset, resolution, frame rate, sample rate, custom ffmpeg args |

Every VEDIT export is a real ffmpeg render with a full filter graph:
per-track layering with opacity, source trimming, speed changes (`setpts` /
`atempo`), per-clip volume, `amix` audio mixing and palette-optimised GIFs.

## Project layout

```
VEDIT/
├── Cargo.toml                  # cargo workspace
├── crates/
│   ├── vedit-core/             # engine: project model, ffprobe, export renderer,
│   │   │                       #         preview decoder, timeline ops
│   │   └── src/{lib,project,timeline,media,probe,ffmpeg,export,preview,thumbnail,error}.rs
│   └── vedit-gui/              # egui app: panels, theme, audio preview
│       └── src/{main,app,theme,audio}.rs + src/panels/*
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
# → installer\Output\VEDIT-Setup-0.1.0.exe   (the installer)
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
& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" /DAppVersion=0.1.0 installer\vedit.iss
# → installer\Output\VEDIT-Setup-0.1.0.exe
```

The installer puts `vedit.exe` **and** `ffmpeg\ffmpeg.exe`/`ffprobe.exe` into
`C:\Program Files\VEDIT`, adds Start-Menu/Desktop shortcuts and a proper
uninstaller. VEDIT auto-finds ffmpeg next to its own exe — nothing to
configure.

Option C — **let GitHub build it for you (zero local setup)**:

1. Push this folder to a GitHub repository.
2. Tag a release: `git tag v0.1.0 && git push origin v0.1.0`.
3. The `release` workflow builds `vedit.exe`, bundles ffmpeg, compiles
   `VEDIT-Setup-0.1.0.exe` and a portable zip, and uploads them as artifacts
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
   speed/opacity/volume.
5. **Space** plays; scrub by dragging the ruler or empty lane.
6. **Export** (Ctrl+E): pick a preset or tune codec/bitrate/CRF/resolution/
   fps yourself, choose the output file, hit **Export** and watch the live
   progress bar (cancel any time).

## Keyboard shortcuts

| Key | Action |
|---|---|
| `Space` | Play / Pause |
| `S` | Split clip(s) at playhead |
| `Del` / `Backspace` | Delete selected clip |
| `←` / `→` | Step 1 frame (`Shift` = 10 frames) |
| `Home` / `End` | Jump to start / end |
| `Ctrl+S` / `Ctrl+E` / `Ctrl+O` / `Ctrl+I` | Save / Export / Open / Import |
| `Wheel` / `Ctrl+Wheel` | Scroll timeline / zoom at cursor |

## Roadmap

- [x] Multi-track timeline with move/trim/split/snap
- [x] Full-ffmpeg export (bitrate/CRF, presets, GIF palette)
- [x] Frame-accurate composited preview
- [ ] Waveform & thumbnail strips on clips
- [ ] Transitions (cross-dissolve) & fade in/out
- [ ] Undo/redo history
- [ ] Keyframed opacity/volume
- [ ] Text & title clips
- [ ] Hardware-accelerated encode (NVENC/QSV/AMF)

## License

MIT — see [LICENSE](LICENSE). ffmpeg is an external binary and is **not**
distributed with this repository; the build scripts download official builds
for you (check each build's licensing: LGPL/GPL).
