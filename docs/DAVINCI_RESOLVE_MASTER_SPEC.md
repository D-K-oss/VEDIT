# DAVINCI RESOLVE 20 — MASTER TECHNICAL SPECIFICATION

**Document class:** Production-grade application blueprint (UI layout, page/sub-tab/panel inventory, feature matrices, settings catalogs).
**Version baseline:** DaVinci Resolve 20 cycle. Studio-only items are marked **[S]**. Free-tier items are unmarked.
**Purpose for VEDIT:** Canonical reference model for parity-driven development. Appendix C maps this blueprint onto VEDIT's implementation priority.

---

## 1. GLOBAL UI FRAMEWORK & WINDOW MANAGEMENT

### 1.1 The Navigation Bar / Page Strip

The page strip is the root navigation control, docked **bottom-center** of the main window (bottom edge on the primary display in dual-screen mode). Each button swaps the entire workspace; every page owns an independent, persistent layout memory (panel sizes, visibility, floating window positions).

| # | Page | Shift-shortcut | Core mission |
|---|------|----------------|--------------|
| 1 | **Media** | Shift+1 | Ingest, clone, metadata, capture |
| 2 | **Cut** | Shift+2 | Fast, single-timeline "edit on rails" |
| 3 | **Edit** | Shift+3 | Traditional NLE page (tracks, trimming, effects) |
| 4 | **Fusion** | Shift+4 | Node-based 2D/3D compositing |
| 5 | **Color** | Shift+5 | Grading, node pipelines, scopes |
| 6 | **Fairlight** | Shift+6 | Digital audio workstation |
| 7 | **Deliver** | Shift+7 | Render queue and publishing |
| 8 | **Photos** | Shift+8 | RAW stills library + grade-powered retouching (newest addition) |

Strip behaviors:
- The strip stays visible on every page except **Video Clean Feed** and full-screen viewer states.
- Page layouts are saved per **project + user**; switching pages never resets panel state.
- The strip's right flank hosts the **Project Settings gear** (§1.4) and the render shortcut; the left flank hosts the active **project name** display.
- **Studio vs. Free:** identical page set — Studio differences are *inside* pages (Neural Engine tools, GPU scopes/NR, 4K+ delivery windows, Atmos, etc.).

### 1.2 Top Menu Bar & Context Menus

On macOS the first menu is the **DaVinci Resolve** menu (About, Preferences `Cmd+,`, Keyboard Customization, Quit). On Windows the menu bar starts at File. The page-agnostic menu set:

| Menu | Representative entries (grouped) |
|------|----------------------------------|
| **File** | Project Manager · New Project · Open Project · Save Project (`Cmd/Ctrl+S`) · Save Project As · Import Project (.drp) · Import Timeline (AAF / EDL / XML) · Import Media (`Cmd/Ctrl+I`) · Export Project · Project Archive (via Project Manager) · Media Management… · Render · Exit |
| **Edit** | Undo (`Cmd/Ctrl+Z`) · Redo (`Shift+Cmd/Ctrl+Z`) · Cut/Copy/Paste (`X/C/V`) · Paste Attributes (`Alt/Opt+V`) · Ripple Delete (`Shift+Del`) · Select All (`Cmd/Ctrl+A`) · Markers submenu (Add `M`, duration, colors, clear) · Segments submenu · Find |
| **Trim** | Ripple Trim Clip Start/End · Roll Edit Forward/Backward · Slip Source In/Out · Slide Clip · Trim to Playhead variants · entry points into the dynamic trim modes (§2.3) |
| **Timeline** | New Timeline (`Cmd/Ctrl+N`) · Generate Timeline From Clips/EDL/Nuendo XML · Track operations (Add/Delete/Rename Track, Track Height presets) · Toggle Snapping · Zoom In/Out · Zoom to Fit (`Shift+Z`) · Timeline Settings (Start Timecode, Format) · Normalize Audio Levels · Detect Scene Cuts |
| **Clip** | Insert (`F9`) · Overwrite (`F10`) · Replace (`F11`) · Append (`F12`) · Open in Fusion Page · New Fusion Clip · New Compound Clip · New Multicam Clip · New Group (color) · Auditions (add takes, next/previous) · Auto Sync Audio (Timecode / Waveform) · Clip Attributes · Retime Controls (`Cmd/Ctrl+R`) · Retime to Duration · Freeze Frame · Enable/Disable Clip |
| **Fairlight** | Audio-track operations · **Bounce Selection to Track / to Bus** · Normalize Audio Levels · Import Audio · Export Audio · ADR / Foley panels (window toggles) |
| **Fusion** | Fusion Settings (cache, threading, OpenGL) · Browse/Save node comps · Macro & template authoring (Save as Macro / as Template) · Select → Group · Comp render controls |
| **Color** | Grab Still · Save Still · Copy This Grade · Paste Grade · Groups (New Pre-Clip / Post-Clip) · Bypass All Grades · Reset Grade / Reset All Grades · LUT import/export · Render Quality (Draft/Half/Full) · Gallery/PowerGrade housekeeping |
| **Workspace** | Layouts (per-page presets, **Save Layout As**, **Reset Layout**) · **Dual Screen On/Off** · **Video Clean Feed** · Full Screen viewer modes · Show/Hide panels (Inspector, Media Pool, Effects, Mixer, Scopes, Index, Metadata…) · Floating windows (Scopes, Key Editor, LUT Browser, ADR, Meters) · Data Burn-In (timecode, clip name, format, watermark) · Keyboard Customization |
| **Help** | About · Product Documentation · Keyboard Customization · Check for Updates |

**Context menus (right-click zones)** — the deep-work surface; each zone exposes page-specific operations:
- **Media Pool clip:** Open in Cut/Edit/Fusion/Photos · Clip Attributes · Detect Scene Cuts · Generate Proxy / Optimized Media · Clone · Copy as hard link · Sync bin grouping · Reveal in Finder/Explorer.
- **Timeline clip:** Cut/Copy/Paste Attributes · Change Clip Color · New Fusion/Compound Clip · Retime & Speed · Split/Insert · Transition apply · Find in Media Pool · Normalize Audio Level.
- **Track header:** Add Track · Rename · Height · Reorder · "Assign track color from clip".
- **Node graph (Color/Fusion):** Add node → Serial / Parallel / Layer / Corrector · Disable · Bypass · Label · Save as macro (Fusion).
- **Render queue job:** Start/Stop/Remove · Reveal output · Copy settings.

### 1.3 Workspace Layout & Dual-Screen Management

- **Panel model:** every page is a docked-panel grid. Splitters resize, headers collapse/expand, and the Workspace menu toggles visibility. Layout state persists per page (per project/user).
- **Layout presets:** per-page factory layouts + user layouts (**Save Layout As**), **Reset Layout** (factory restore) — the first remedy for "broken UI" states.
- **Dual screen:** Workspace → **Dual Screen On/Off** redistributes panel sets to monitor 2 per-page (classic grading suite: viewer + scopes + nodes on the grading monitor; source viewer + media pool on the second).
- **Video Clean Feed:** strips 100% of UI chrome and shows the program viewer only (main display and/or SDI/HDMI monitor) for client review.
- **Full-screen viewer modes** + **Auto-hide UI** behaviors for playback review.
- **Vertical layout:** dedicated UI arrangement for 9:16 / vertical-format projects (panel stacks reorder for portrait video) — Workspace → Layout.
- **Floating windows:** Scopes, Key Editor (marker/note), LUT Browser, ADR panel, Fairlight meter bridge, Fusion viewers, Media Pool (tear-off bins) — any can live on the second display.

### 1.4 Project Settings Hub

Two distinct dialogs; both reachable from the page-strip gear cluster.

#### 1.4.1 Project Settings (per project — gear icon, bottom-right)

| Tab | Contents |
|-----|----------|
| **Master Settings** | **Timeline Format:** resolution (presets HD → 8K + custom, incl. vertical formats), pixel aspect, playback frame rate, interlacing mode, **Video Data Levels** (Video / Full / Data), timeline color space. **Video Monitoring:** monitor format, monitor data levels, HDR output / Dolby Vision monitor options, "Use 10-bit precision on monitors". |
| **Capture** | Capture preset (codec/format per device), video input, audio channel count & mapping (2/4/8/16), timecode source, capture filename prefix/reel, RS-422 deck control (legacy tape). |
| **Image Scaling** | **Input scaling rules:** separate policies for "clip resolution **larger** than timeline" and "smaller than timeline" — each set to *Scale to fit / Fit / Fill / Stretch / Center crop with zoom*; **Super Scale quality** default (Off / 2x / 4x **[S]**); output blanking enforcement. |
| **Color Science / Color Management** | Mode: **DaVinci YRGB** (unmanaged, legacy) · **DaVinci YRGB Color Managed (RCM)** · **ACES**. RCM: working space (default **DaVinci Wide Gamut / DaVinci Intermediate**), input color space tags (auto/manual per format), output color space + gamma, tone-mapping controls, HDR mastering settings (luminance, middle grey, white point). ACES: version (1.0–1.3), working space (**ACEScc / ACEScct / ACEScg**), output transform (SDR Rec.709, P3, PQ/HLG HDR variants), IDT override behavior. |
| **Camera RAW** | Default decode quality (Full/Half/Quarter per format family), **Decode Using: Clip vs. Project**, white balance / exposure defaults ("as shot" vs. custom), RAW noise reduction, per-format color space tag & gamma tag feeding RCM. |
| **Fairlight** | Project **sample rate** (default 48 kHz; up to 192 kHz), audio process buffer behavior, automation safety defaults. |

#### 1.4.2 Preferences (application-wide — File → Preferences / `Cmd/Ctrl+,`)

Organized as **User** and **System** top-level tabs with a panel list:

| Panel | Key controls |
|-------|--------------|
| **Media Storage** | Database location (local disk default), volume list with "show unmounted", proxy/optimized-media generation defaults, cache size/location. |
| **Capture and Playback** | Capture card selection, video/audio input routing, capture codec preset, device timecode, remote deck. |
| **Video and Audio I/O** | Monitor output format/data levels, HDR monitor handshake, audio output device and buffer. |
| **Control Panels** | Detected panels: DaVinci Resolve Editor Keyboard / Speed Editor (USB+Bluetooth), Micro/Mini/Major Panel, Fairlight hardware; mapping and connection status. |
| **Project Save and Load** | **Live Save** toggle, project backup count/interval, backup folder path. |
| **Language and Keyboard** | UI language; keyboard layout presets (DaVinci + NLE-emulation sets) and the full **Keyboard Customization** editor. |
| **UI Settings** | Viewer/UI detail options (thumbnail styles, timeline appearance, high-contrast text, on-screen control defaults). |
| **Fusion / Fairlight housekeeping** | Cache sizes, plugin scan paths (VST/AU manager), playback buffer defaults. |

---

## 2. DEDICATED PAGES — COMPONENT-BY-COMPONENT BREAKDOWN

### 2.1 MEDIA PAGE

**Layout (four zones):**

| Zone | Contents |
|------|----------|
| **Left — Media Storage** | Volume tree, file browser (list/thumbnail), favorites/recents; **Clone Tool** launch; preview strip. |
| **Center/Left — Media Pool** | Bin tree (nested bins, **Smart Bins** with rule-based queries), view modes (Thumbnail / Thumbnail+Metadata / List with customizable columns), search, sort, clip color labels, bin locking indicator (collab). |
| **Top-right — Viewer** | Source preview with transport (`J/K/L`, `I/O`, frame step), audio scrub, metadata overlay toggle. |
| **Bottom-right — Metadata** | Field grid (scene/take, reel, custom fields); field-list manager for adding custom metadata columns. |

**Feature matrix:**

| Feature | Behavior |
|---------|----------|
| **Ingest** | Drag-to-pool import; **Media Management** dialog (move/copy/transcode/relink across volumes without breaking links). |
| **Clone Tool** | Checksum-verified duplication (MD5 / xxHash64) preserving folder structure, with copy log — offload safety. |
| **Reel naming** | Reel field per clip (metadata + bin) — the key consumed by EDL conform. |
| **Timecode manipulation** | **Clip Attributes → Timecode:** start TC override, timecode format, duration recalculation. |
| **Audio channel mapping** | Clip Attributes → Audio: channel layout assignment (mono/stereo pairs/8-ch maps) and per-channel enable. |
| **Live capture** | Log and Capture window: device preview, record-to-disk with the Capture preset, TC-accurate file naming. |
| **Scene Cut Detection [ML]** | Analyzes long takes and splits them into discrete clips in a bin (speeds multicam/rough-cut). |
| **Sync bins** | Group clips into multi-cam collections by **timecode or audio waveform** (feeds Cut page Sync Bin). |
| **Proxy / Optimized media** | Context-menu generation; "Prefer proxy media" resolution policy. |

### 2.2 CUT PAGE

**Layout:** single-screen, top-to-bottom: **Viewer** (source-tape default) → **transitions strip** → **Dual Timeline** → media strip. Media Pool appears as a left dock; Inspector pops contextually right.

| Zone | Contents |
|------|----------|
| **Viewer** | **Source Tape** mode: all bin clips spliced into one virtual reel with filmstrip scrub; standard source mode with In/Out; multicam **Sync Bin** grid mode; **Fast Review** button; on-viewer timecode. |
| **Dual Timeline** | **Upper strip** = master overview of the whole edit (always fully visible; click-to-navigate, never zoomed); **lower timeline** = detail, **magnetic** (trims and edits ripple; gaps self-heal). |
| **Transitions strip** | One-click dip/blend/transition application at the playhead cut. |
| **Quick Export** | Top-right button: render immediately with a chosen preset (no page change). |

**Feature matrix:**

| Feature | Behavior |
|---------|----------|
| **Smart Insert `Q`** | Insert source at playhead on the lowest free track space, ripple forward. |
| **Ripple Overwrite `W`** | Overwrite at playhead; downstream media ripples to close the gap. |
| **Append `E`** | Splice source to the timeline tail. |
| **Source Overwrite** | Mark In/Out on the **Source Tape**, drop into the timeline — the signature "search once, cut fast" move. |
| **Fast Review** | Variable accelerated playback that keeps dialogue intelligible — rapid footage triage. |
| **Sync Bin** | Multi-cam angle grid synced by TC/waveform; live angle-switch during playback writes edits. |
| **Fast Trim** | Edge trims ripple instantly; no trim-mode switching (magnetic model). |
| **Smart Reframe [S][ML]** | Subject-tracking reframe 16:9 → 9:16/4:5/1:1 in the Cut Inspector. |
| **Cut Inspector** | Transform (rotate/zoom/position), speed change, Smart Reframe panel. |

### 2.3 EDIT PAGE

**Layout (classic NLE five-zone):**

| Zone | Contents |
|------|----------|
| **Left** | **Media Pool** (bins, smart bins, search, view modes) + **Effects Library** tree: *Toolbox* (Generators, Solids, Titles), *ResolveFX* groups (Color / Blur / Generate / Light / Stylize / Texture / Beauty), *Video Transitions* (Cross Dissolve, Dip to Color, Push/Slide/Wipe family, **Smooth Cut [S]**, Fusion transitions), *Titles* (**Text**, **Text+** Fusion titles, Fusion title packs), *Audio FX* (Fairlight FX + hosted VST3/AU), *Audio Transitions*. |
| **Top-center** | **Dual Viewer** — Clip/Source viewer (left) + Timeline viewer (right): transport `J/K/L`, `I/O` marks with In/Out range flags, shuttle ring, frame step, loop, timecode displays, viewer channel/aspect toggles. |
| **Right** | **Inspector** — tabs **Video | Audio | Subtitles | Transition** (+ OpenFX parameter stack when an effect is selected). |
| **Bottom** | **Timeline**: toolbar (**A** pointer, **B** blade, **T** trim-edit mode, snapping, linked selection, marker add, zoom), track headers (video: lock/output-enable/height; audio: M/S/lock/height/stereo badge), timecode ruler with In/Out shading, clips (filmstrip thumbnails on video, waveforms on audio, fade handles, keyframe overlays, clip-color labels), playhead, minimap-style navigation bar. |

**Inspector — Video tab (full section list):**

| Section | Parameters |
|---------|-----------|
| **Transform** | Position X/Y · Zoom X/Y · Rotation angle · Anchor Point · **Pitch / Yaw** (2.5-D perspective) · Flip Horizontal/Vertical. |
| **Cropping** | Left / Top / Right / Bottom soft-edge crop. |
| **Dynamic Zoom** | Keyframable push/pull with Ease In/Out presets (quadratic/smooth). |
| **Composite** | Composite mode (Normal, Add, Subtract, Screen, Overlay, Soft Light… 20+ blend set) · Opacity. |
| **Retime & Speed Change** | Speed % · frame interpolation: **Nearest / Frame Blend / Optical Flow / Optical Flow + Speed Warp [S]** · reverse · ripple timeline option. |
| **Stabilization** | Mode: **Perspective / Similarity / Translation** · Smooth strength · Zoom (Crop / Reflect / No zoom) · per-frame analysis. |
| **Lens Distortion** | Model + a/b/c coefficients, apply before/after transform. |
| **Super Scale [S]** | Off / **2x / 4x** upscale with detail recovery; sharpen/noise sub-controls. |
| **HDR tone controls** | When timeline/project is HDR: exposure/contrast in stops. |

**Inspector — Audio tab:** Volume · Pan (and dual-balance) · per-clip **EQ** · **Pitch** (semitones + cents) · **AI Dialogue Leveler** (on/off + intensity) · channel gain.

**Inspector — Subtitles tab:** subtitle style presets (font/size/color/stroke/shadow/background), safe-area guides, per-caption styling.

**Inspector — Transition tab (when an edit point is selected):** duration · alignment (Center/In/Out) · the transition's own parameters (e.g., Smooth Cut sensitivity, Dip color, Fusion transition inputs).

**Feature matrix:**

| Feature | Behavior |
|---------|----------|
| **Three-point editing** | Source In/Out + timeline target → **Insert `F9` / Overwrite `F10` / Replace `F11` / Append `F12`**; Replace retains timing. |
| **Trimming modes** | **Roll, Ripple, Slip, Slide** via Trim menu / dynamic trim; **Trim Edit Mode `T`** + `J/K/L`-scrub = **dynamic trimming** (variable-speed trimming while playing). |
| **Multicam editing** | New Multicam Clip (sync by TC or waveform) → multicam viewer grid → live angle switching during playback writes cuts; flatten/expand the multicam inside the timeline; angle banking; **Multicam SmartSwitch [S][ML]** auto-cuts to the active speaker. |
| **Speed ramps** | Retime Controls `Cmd/Ctrl+R`: speed points, retime curve editor, freeze frame, "Retime to Duration"; Speed Warp interpolation **[S]**. |
| **Titles** | Text (2-D) and **Text+** (Fusion node template with published inputs); Fusion title packs appear in Effects Library. |
| **Subtitles/captions** | Subtitle track: import SRT/ITT/EDL, style in Inspector, export embedded or burned (Deliver). |
| **Text-Based Editing [transcription]** | Transcribe timeline audio → editable transcript; strike text to cut, remove pauses automatically; select words → replace insert; **IntelliScript [ML]** builds a timeline edit by matching a script against media transcripts. |
| **Audio sync** | Auto Sync Audio by **timecode** or **waveform**; link/unlink A/V; merged clips. |
| **Compound / Fusion clips** | New Compound Clip (nest), New Fusion Clip (multi-layer comp container), both re-enterable. |
| **Auditions** | Stack alternate takes on one clip; cycle takes in place without timeline disturbance. |
| **Adjustment clips** | Empty layer carrying grade/blanking/composite applied to everything below (track-like grading). |
| **Markers** | Colors, durations, notes, marker panel list, `M` to add, `Shift+M` cycles. |
| **Timeline navigation** | Zoom to Fit `Shift+Z`; playhead follow; track height presets; In/Out duration readouts. |

### 2.4 FUSION PAGE

**Layout:**

| Zone | Contents |
|------|----------|
| **Center** | **Node Editor** — free-form graph canvas: MediaIn → tool nodes → MediaOut; box/group nodes; unlimited branching. |
| **Left** | **Toolbar** (tool palette by category): Tools (Transform, Color Corrector, Blur, Sharpen, Keyers, Mattes, Merge, Warp) · Particles (pEmitter, pTurbulence, pRender…) · 3-D (Camera3D, Light3D, Merge3D, Shape3D/Renderer3D, CameraTracker) · Paint · Tracker · Windows. |
| **Top** | **Dual Viewers** — any node can be viewed (A/B splits, wipe checks) plus the comp result. |
| **Right** | **Inspector** (selected tool; modifiers; published inputs) + **Spline Editor** + **Keyframes Editor** (dope-sheet lanes per parameter). |
| **Bottom** | **Comp timeline** — playhead + keyframe lanes; drag keyframes; render range. |

**Feature matrix:**

| Feature | Behavior |
|---------|----------|
| **Per-clip comps** | Edit-page clip → "Open in Fusion Page" wraps it in MediaIn/MediaOut nodes; composite is live in the Edit timeline. |
| **2-D compositing** | Serial/parallel node graphs, Merge with blend operations, masks/roto (Polygon / B-Spline / Bezier soft shapes), **Paint** vector strokes (clone/paint-out). |
| **Keying** | **Delta Keyer** (clean-plate subtraction, matte finesse: shrink/grow/soften, spill suppression, edge treatment), **Ultra Keyer**, Chroma/Luma keyers. |
| **Tracking** | **Tracker** node (Point / Perspective / Planar modes), **PlanarTracker**, **CameraTracker** (3-D camera solve from footage, point cloud + lens calibration). |
| **3-D workspace** | Cameras, lights (point/spot/directional), geometry primitives + **FBX / Alembic / USD** import, Merge3D scene assembly, Renderer3D (software / OpenGL / GPU), depth-of-field. |
| **Particles** | 2-D and 3-D particle systems (emitters, turbulence, renderers, region forces) — node-graph composable. |
| **Optical flow** | TimeStretcher / OF-based retiming; Speed-Warp-quality interpolation shares the optical-flow engine **[S]**. |
| **Animation system** | Per-parameter keyframing, **Spline Editor** (Bezier / eased / stepped interpolation), **expressions** (math links between parameters), modifiers (connect a tracker's output to any Transform), Publish inputs for template exposure. |
| **Macros & templates** | Select nodes → **Save as Macro** (expose chosen inputs) → appears in the Edit-page Effects Library as Title/Generator/Transition template. |

### 2.5 COLOR PAGE

**Layout:**

| Zone | Contents |
|------|----------|
| **Top** | **Thumbnail Timeline** — filmstrip of timeline clips; group/rod indicators; still-frame thumbnails for graded state; navigation arrows + lighttable mode. |
| **Center** | **Viewer** with on-screen tools (zoom/pan, framing guides, window overlays when active). Below the viewer, the **tool strip**: **Qualifiers (HSL / RGB / Luma / 3-D) · Power Windows · Tracker · Magic Mask [S] · Scopes · Spatial NR [S] · Temporal NR [S]**. |
| **Below** | **Primaries panel** — tabs: **Color Wheels · Primaries Bars · Curves · HDR Color Wheels [S] · Color Match · Camera RAW · ColorSlice (six-band vector corrector) · FX (OpenFX)**. |
| **Right** | **Gallery** (Stills albums + **PowerGrades**) · **Node Editor** · **Clips panel** · **Scopes panel** · **Inspector** (Sizing / Stabilization / clip data). |

**Primaries panel detail:**

| Tab | Controls |
|-----|----------|
| **Color Wheels** | **Lift / Gamma / Gain / Offset** — each a hue-ring + master wheel + vertical slider; **Log mode** swaps to Shadow/Midtone/Highlight/Offset; luma-only (Y) toggle for wheel moves. |
| **Primaries Bars** | RGB Lift / Gamma / Gain bars + Luma bars (printer-light style). |
| **Curves** | **Custom** (master + R/G/B channels) · **Hue vs Hue · Hue vs Sat · Hue vs Lum · Sat vs Sat · Lum vs Sat** spline curves. |
| **HDR Color Wheels [S]** | Shadow/Mid/Highlight wheels tuned for PQ/HLG ranges + master Offset; HDR grading sliders (black offset, highlight rolloff…). |
| **Color Match** | Auto-match two clips (reference still or live), or chart-assisted matching (color-chart detection). |
| **Camera RAW** | Per-clip RAW: decode quality, WB/tint, exposure, decode-using, color-space/gamma tags. |
| **ColorSlice** | Six-band hue-vector corrector: grab a hue range, then per-band sat/gain/offset — fast "split-tone by hue" work. |
| **FX** | ResolveFX stack for the current clip (same plugins as Edit page, applied in the grade chain). |

**Feature matrix:**

| Feature | Behavior |
|---------|----------|
| **Node pipeline** | Serial corrector chains; **Parallel** (ParallelMixer) and **Layer** (LayerMixer, composite-priority) nodes; **Splitter/Combiner** for channel-split grades (RGB / YCbCr / HSL); node disable/bypass/labels; unlimited nodes per clip. |
| **Groups** | **Pre-Clip → Shared Group → Post-Clip** node trees: shared grade across grouped clips with per-clip pre/post latitude. |
| **Secondary grading** | **Qualifiers:** HSL (hue/sat/lum ranges + denoise/blur + in/out tolerance), RGB, Luma, 3-D (volumetric color pick); **matte finesse** (shrink/expand, soften, edge repair); invert; combine with **Power Windows** (Circle/Square/Polygon/Gradient/PowerCurve, on-screen handles, softness, combine modes) and **Tracker** follow (Point / Perspective / Planar). Inside/Outside grades per window. |
| **Magic Mask [S][ML]** | Stroke subject/background → ML isolation (person: hair/face/body; objects) that tracks through the shot. |
| **Tracker & Stabilization** | Tracker panel (point/perspective/planar, forward/backward analysis, smooth); Stabilization in Inspector (perspective/similarity/translation + smooth strength + zoom crop). |
| **Gallery & grades** | Grab **Stills** into albums; **PowerGrades** persist across projects (drag onto clips); wipe/AB compare against stills or live; grade **versioning** (A/B/C… per clip); copy/paste grade; save grade as file. |
| **Scopes** | **Waveform (Y / RGB / YRGB)** · **RGB Parade** · **Vectorscope** (75%/100% graticules + target) · **Histogram** · **CIE chromaticity** (gamut-boundary overlays: Rec.709 / P3 / Rec.2020) · **3-D color cube scope**; multi-scope layouts on any monitor. |
| **Noise reduction [S]** | **Spatial NR** (frame-local, edge-aware) and **Temporal NR** (inter-frame motion-compensated) as panel tabs with threshold/blend controls. |
| **Dolby Vision [S]** | Shot analysis, trim passes (e.g., 100-nit/600-nit), metadata export with the render. |
| **HDR grading** | HDR wheels + HDR mastering settings (Project Settings) + clipping indicators; HDR scopes. |
| **Remote/collab grading** | Color Trace across timelines/projects; shared cloud projects for remote grading sessions. |

### 2.6 FAIRLIGHT PAGE

**Layout:**

| Zone | Contents |
|------|----------|
| **Top** | **Audio timeline** — sample-accurate editing (zoom to sample level), track headers with meters, waveform clips, **automation lanes** (volume/pan/mute), fade handles and crossfade shapes, clip gain overlays. |
| **Left** | **Track Index** — track list with index numbers, kind badges (A / V / B / Subtitle), M/S/R states, reorder. |
| **Top strip** | **Meter bridge** — per-track and per-bus meters with peak hold, configurable scale, loudness readouts. |
| **Bottom** | **Mixer** — channel strips for every track + **bus tracks** + master strip. |
| **Right** | **Inspector** (clip volume/pan/gain/fades) + **Effects rack** (per-track insert list). |

**Mixer channel strip (top→bottom):**

| Strip section | Controls |
|---------------|----------|
| **Input** | Trim gain, phase invert, input routing. |
| **Inserts 1–10** | FX slot chain (Fairlight FX + VST3/AU) in processing order. |
| **EQ** | Mini response graph + expand: 6-band parametric (band type/freq/Q/gain per band). |
| **Dynamics** | Mini transfer graph + expand: **Fairlight Dynamics** (expander/gate → compressor → limiter) with full parameter set. |
| **Sends** | Aux send level per bus (pre/post fader). |
| **Pan** | Stereo pan / surround / 3-D spatial panner. |
| **Fader** | dB-scaled fader, 0 dB detent, dB readout, automation mode selector (Off/Read/Touch/Latch/Write). |
| **Strip switches** | S · M · **R (record arm)** · monitor/cue enable. |
| **Output routing** | Track → bus / Main assignment matrix. |

**Feature matrix:**

| Feature | Behavior |
|---------|----------|
| **Fairlight Audio Core** | Unified **32-bit float** processing at project sample rate; up to **2,000 tracks** per timeline; sample-accurate clip placement (frame grid-free). |
| **Recording** | Punch-in/punch-out recording with preroll; input monitoring; record-safe interlocks with the R states. |
| **ADR [S]** | ADR panel: cue list (In/Out/Dur, character/line, take count), **streamers + beeps** (3-pop), take recording with ratings, cycle takes, auto placement at cue. |
| **Foley** | Foley recording window with picture guide, multi-take stacking per cue. |
| **EQ / Dynamics suite** | 6-band EQ, Fairlight Dynamics (gate/comp/limiter), limiter, de-esser, dialogue processor, multiband — as inserts or per-clip. |
| **Plugin hosting** | **VST3** (Windows/macOS) + **AU** (macOS) with plugin manager (scan paths, enable per project). |
| **AI tools** | **Voice Isolation [S]** (strip dialogue from noise), **AI Dialogue Leveler** (even out dynamics), **Dialogue Separator** (split dialogue/background into stems), **Music Remixer [ML]** (rebalance vocals/drums/bass/other), **AI Audio Assistant [20]** (proposes routing + processing for dialogue/music buses). |
| **Immersive / Atmos** | Bed + **object tracks** (up to 118 objects + beds to 9.1.6), 3-D panner, binaural headphone monitoring, **ADM BWF import/export**, integrated Dolby Atmos renderer. |
| **Buses** | Main + aux buses of any type (mono/stereo/5.1/7.1/Atmos); **bus tracks** appear in the timeline/mixer with their own strips; cue buses for talent. |
| **Automation** | Volume/pan/mute automation with Off/Read/Touch/Latch/Write; automation preview and safe-write modes. |
| **Metering** | Peak/RMS meters, **loudness meter (M/S/I LUFS + true peak)**, correlation meter, downmix checks. |
| **Normalize / Bounce** | Timeline Normalize Audio Levels (peak or **LUFS target**); **Bounce selection to track/bus** (render-in-place). |

### 2.7 DELIVER PAGE

**Layout:** left **Render Settings** (preset list + tabs when Custom), right **Render Queue**.

**Render Settings — Custom Export tabs:**

| Tab | Contents |
|-----|----------|
| **Video** | **Format** (MP4 / MOV / MXF OP1A / AVI / individual-frame image sequences) · **Codec** (matrix below) · Resolution & scaling (custom W/H, fit policies) · **Frame rate override** · **Quality mode:** quality-based slider / average bitrate (1-pass, 2-pass) / constraint · keyframe interval · **hardware encode** toggle (NVENC / AMF / QSV / VideoToolbox where present) · **Advanced:** data levels (Video/Full), color-space tag & gamma tag, alpha export (single/multi for 4444-family), HDR/Dolby Vision metadata, network optimization (web-faststart MP4). |
| **Audio** | Codec (AAC / Linear PCM / MP3) · data rate · channels (auto / 2 / 5.1…) · sample rate · **loudness normalization to LUFS target** option. |
| **File** | Filename template (project name / timeline name / tokens) · output location · **unique filename auto-increment** · **export timeline markers as chapters** (YouTube/Vimeo) · file-per-clip naming policies. |
| **Render source** | **Entire Timeline** · **Individual Source Clips** (each graded source re-rendered, for conform/round-trip) · In/Out range. |

**Codec matrix (primary set):**

| Codec family | Bit depth / chroma | Notes |
|--------------|--------------------|-------|
| **H.264** | 8-bit 4:2:0 | SW + hardware encoders; the social/default delivery codec. |
| **H.265 / HEVC** | 8/10-bit 4:2:0 | HW-accelerated encode where available; HDR-friendly. |
| **AV1** | 8/10-bit | Modern royalty-free delivery target. |
| **Apple ProRes** | 10-bit 422 (Proxy/LT/422/HQ), 12-bit 4444 / 4444 XQ | Mastering/post interchange; alpha in the 4444 family. |
| **DNxHD / DNxHR** | 8/10-bit (LB/SQ/HQ/HQX/444) | Avid-house interchange. |
| **Uncompressed** | 10-bit YUV / RGB | Realtime SAN workflows. |
| **Image sequences** | DPX 10/12-bit log · EXR 16F/32F (none/PIZ/ZIP/DWAA…) · TIFF 8/16 · PNG · TGA · JPEG | VFX round-trips, absolute-quality masters. |

**Render Queue:**

| Element | Behavior |
|---------|----------|
| Job list | Name, settings summary, progress, estimated size/time, status; reorder; remove. |
| Start/Stop | Sequential queue execution; pause/resume; background render policy. |
| Multi-job | Same timeline rendered at multiple formats/resolutions as separate jobs; custom presets saved/shared. |
| **Publishing** | Direct upload **YouTube / Vimeo / Frame.io [S]** with account linking; marker chapters export with the upload. |

### 2.8 PHOTOS PAGE (newest addition)

Specialized **RAW still-image** workspace riding the same engine (Camera RAW decode, RCM color pipeline, Deliver render stack).

| Zone / feature | Behavior |
|----------------|----------|
| **Library grid / albums** | RAW files (CR3 / NEF / ARW / DNG…) browsed as a thumbnail grid; album organization; rating/color labels. |
| **Viewer + grade tools** | Full Color-page toolset (wheels, curves, ColorSlice, LUTs) applied non-destructively to stills. |
| **Batch processing** | Apply grade / exposure / crop across album selections; sync settings across selections. |
| **Exposure compensation** | Stops-based exposure, WB/tint, highlight/shadow recovery — via the Camera RAW engine. |
| **Crop / reframe** | Aspect-ratio constrained crop with safe guides; rotation and straighten. |
| **Retouching** | Spot/blemish removal and local corrections layered on the grade (grade-engine powered). |
| **Export** | JPEG / TIFF / PNG / DNG variants through the Deliver codec stack, including resolution presets. |

---

## 3. NEURAL ENGINE & ECOSYSTEM

### 3.1 Neural Engine — cross-page automation matrix

| Feature | Page(s) | Function | Tier |
|---------|---------|----------|------|
| **Magic Mask** | Color | ML subject/object isolation (hair/face/body/objects), auto-tracking windows | **[S]** |
| **Speed Warp** | Edit | Optical-flow-quality retiming via ML motion estimation | **[S]** |
| **Voice Isolation** | Fairlight | Remove background noise from dialogue | **[S]** |
| **AI Dialogue Leveler** | Edit / Fairlight | Flatten dialogue level dynamics | Free |
| **Dialogue Separator** | Fairlight | Split dialogue / background / noise into stems | **[S]** |
| **Music Remixer** | Fairlight | Rebalance vocals / drums / bass / other inside mixed music | **[S]** |
| **Super Scale** | Edit | 2x / 4x detail-recovering upscale | **[S]** |
| **Smart Reframe** | Cut | Subject-tracking reframing to 1:1 / 4:5 / 9:16 | **[S]** |
| **Scene Cut Detection** | Media | Detect and split shots inside long clips | Free |
| **Auto Color Match** | Color | Match clip colors to a reference/chart | Free |
| **IntelliScript** | Edit | Build the timeline edit by matching a script to transcript + media | 20 |
| **Animated Subtitles** | Edit | AI transcription with animated, stylized caption presets | 20 |
| **Multicam SmartSwitch** | Edit / Cut | Auto multicam angle switching to the active speaker | **[S]** / 20 |
| **Video Deep Search** | Media | Semantic footage search in bins by content | **[S]** / 20 |
| **AI Audio Assistant** | Fairlight | Automatic dialogue/music bus routing, leveling and processing setup | 20 |
| **Face Refinement** | Color (ResolveFX) | ML face tracking + targeted enhancement | **[S]** |

### 3.2 Project Libraries, Databases & Blackmagic Cloud

| Layer | Options & behavior |
|-------|--------------------|
| **Project storage** | **Local disk database** (default, per-machine) · **DaVinci Resolve Project Server** (database server for teams, simultaneous access) · **Blackmagic Cloud Library** (cloud-hosted project storage). |
| **Live collaboration** | Multiple users in the same live project across pages; presence/position indicators; bin locking; timeline-merge safety. |
| **Blackmagic Cloud** | Project sync across machines; **Presentations** — share a timeline link for remote review with timecoded annotations; media strategies with Blackmagic Cloud Store. |
| **Remote monitoring [S]** | Stream the program output to remote clients (browsers/tablets) with scope overlays for approval sessions. |
| **Interchange** | AAF (Avid/Pro Tools), EDL (CMX3600), FCPXML/XML, ALE metadata, SRT/ITT captions, LUTs (.cube etc.), Dolby Vision metadata XML, ADM BWF. |

---

## APPENDIX A — CORE KEYBOARD SET (default layout)

| Context | Shortcut | Action |
|---------|----------|--------|
| Global | `Shift+1…8` | Switch page (Media…Photos) |
| Global | `Cmd/Ctrl+,` | Preferences |
| Playback | `J / K / L` | Reverse / pause / play (multi-tap for speed) |
| Playback | `I / O` | Mark In / Out · `Alt/Opt+I/O` clears |
| Playback | `← / →` | Frame step (+`Shift` for 10-frame jumps) |
| Timeline | `Shift+Z` | Zoom timeline to fit |
| Timeline | `M` | Add marker · `Shift+M` cycle markers |
| Edit tools | `A / B / T` | Pointer / Blade / Trim-Edit mode |
| Conform | `F9 / F10 / F11 / F12` | Insert / Overwrite / Replace / Append |
| Cut page | `Q / W / E` | Smart Insert / Ripple Overwrite / Append |
| Retime | `Cmd/Ctrl+R` | Retime controls |
| History | `Cmd/Ctrl+Z` · `Shift+Cmd/Ctrl+Z` | Undo / Redo |

## APPENDIX B — FREE vs. STUDIO (headline deltas)

| Category | Free | Studio |
|----------|------|--------|
| Neural Engine | Dialogue Leveler, Scene Cut, Auto Color Match, IntelliScript-class basics | Magic Mask, Speed Warp, Voice Isolation, Super Scale, Smart Reframe, Music Remixer, SmartSwitch, Deep Search |
| Color | Full primaries/secondaries, node graph, gallery | GPU scopes/NR, HDR wheels, Dolby Vision, remote grading |
| Fairlight | Full DAW, VST/AU, LUFS metering | ADR, Voice Isolation, Atmos immersive tooling |
| Deliver | Up to UHD delivery windows, all core codecs | 4K+ windows, DCI/finishing extras, Frame.io upload |
| Interop | AAF/EDL/XML import/export | Same + advanced remote/collab features |

## APPENDIX C — VEDIT IMPLEMENTATION PRIORITY MAP

Derived from this blueprint against VEDIT 0.4.0 (already shipped: Edit-page core, Fairlight EQ/dynamics/LUFS/meters/marker index, stereo waveforms + automation line, overview minimap, 6-band EQ, dynamics insert chain, proxy pipeline).

| Priority | Blueprint target | VEDIT gap (highest leverage first) |
|----------|------------------|-------------------------------------|
| **P0** | §2.3 Edit — Inspector depth | Full Video-tab section set (Transform/Pitch-Yaw/Crop/Composite modes/Speed interpolation selector), Subtitles tab wiring (subtitles.rs exists unwired in HEAD) |
| **P0** | §2.7 Deliver | Codec matrix expansion + queue polish (ProRes/DNxHR/image-sequence export presets, loudness normalization option, marker-chapter naming) |
| **P1** | §2.2 Cut behaviors | Magnetic dual-timeline page shell (upper overview + lower detail), Q/W/E smart edits, transitions strip |
| **P1** | §2.6 Fairlight | Automation lanes on the audio timeline (write volume automation from the fader), send/bus routing matrix, FX-rack parity via Fairlight-FX-style inserts |
| **P2** | §2.5 Color | ColorSlice-style six-band hue corrector; qualifier-lite (luma key) + window shapes on the grade panel |
| **P2** | §1.1/1.3 | Photos page shell (RAW stills grid + grade tools reusing colorgrade.rs) |
| **P3** | §2.4 Fusion | Out of architectural scope for VEDIT's single-binary model — permanent roadmap item |
