# VIctory on an RX 7900 XTX

A fork of [VIctory](https://github.com/Marvisak/VIctory): a compatibility layer
that runs an Xbox Series title natively on Windows from files the user supplies.
This fork works on AMD RDNA3 (RX 7900 XTX) and keeps its numbers on one PC, so
that a change is judged against the same machine and the same scene.

**This repository contains no game files, code or assets.** You need your own
copy of the game. It is not affiliated with or endorsed by the game's developer
or publisher.

## The PC these numbers come from

| | |
|---|---|
| CPU | Ryzen 7 9800X3D |
| Memory | 32 GB |
| GPU | Radeon RX 7900 XTX, 24 GB |
| OS | Windows 11 x64 |
| Title build | 1.0.2.1, Series X profile, ray tracing off, 3840x2160 output |

## Where it stands (2026-11-28)

Heavy frames in the open world, Series X profile, the tuned launcher
(`run-tuned.bat`), the title's own upscaler:

| | |
|---|---|
| Frame | **43-49 ms** (about 21-23 fps), 5,900-7,000 draws a frame |
| Light scenes | 33-35 ms, against the title's own 30 fps limit |
| Preparing, submitting thread | 25.0 ms, of it copies 7.3 ms / 33 MB and texture refreshes 0.9 ms |
| Copying back | **1.3 ms / 1.3 MB** |
| The card | 5.5 ms |
| The title's own work | about 17 ms, not covered by any meter of ours |

That last line is the one to know: a 1 ms stack sampler
(`VICTORY_PROFILE=<from>,<to>,1,<file>`) puts 38% of all samples on one wait,
and the frames under it are the title's own - `ntdll+0x166464` <
`KERNELBASE+0x1427f` (`WaitForSingleObject`) < `victory-image-*.dll+0x221c3c2`,
with no frame of any victory module anywhere in the eight frames below it. The
title is running its own job graph and its main thread waits for its workers.
That is the floor this layer sits on; shortening it means giving the title fewer
jobs, which is what the copy-back work below did.

## What got it there

Measured on this PC, alternating arms in one session, heavy frames only.

| Change | Effect |
|---|---|
| The five renderer ports (below) | 59.0 -> 44.0 ms |
| [PR #477](https://github.com/Belobug/VIctory-Belobug-AMD/pull/477) `VICTORY_WC_NO_BACK=1`: shader writes into write-combined title memory that are 64 KiB or more stay on the card, with no snapshot, no compare and no copy-back | 53.3 -> 43.6 ms; copying back **18.1 -> 1.3 ms**, 22.8 -> 1.1 MB; copies up 63 -> 34 MB |
| `VICTORY_RELAY=2` with `VICTORY_WIDE_BACK=1` (the launcher's defaults) | the submission thread stops waiting on per-flight work |

The ports, all on in the launchers (`scripts\play.ps1` sets them):

| Switch | What it does |
|---|---|
| `VICTORY_PORT_CONSTANTS=1` | The title's 174 MB constants heap is read by the card where the title writes it: no uploads, no fills |
| `VICTORY_PORT_DESCRIPTORS=1` | The title's descriptor-table stores are patched (byte-checked) to report the slots they write, instead of a scan of ~600,000 slots a frame |
| `VICTORY_PORT_CBV=1` | Translated shaders read constant buffers in place as raw bytes (~22,000 fills a frame to none; -4.7 ms) |
| `VICTORY_PORT_GUARD=1` | The write guards' lock is shared by faults and refreshes, with a claim per 64 KiB block; the submission's planning runs beside it (title threads' waits 29 ms to ~2 ms a frame; -2 ms) |
| `VICTORY_PORT_JOURNAL=1` | The heaps' records are asked for at the title's submit, and the journal's notes are split into 64 parts with locks of their own (the submit thread's waits ~5 ms + 1 ms to ~0.3 ms; -1 to -3 ms) |

## What was measured and did not help

Every one of these was run as an A/B in one session, and every one is recorded
with its numbers in [`docs/tuned-2026-11-27.md`](docs/tuned-2026-11-27.md).

| Tried | Result |
|---|---|
| Output at 1920x1080 (renders 1280x720) | -3 ms while the copy-back was 18 ms; **with `WC_NO_BACK` it reverses** (copying back comes back as 18-21 ms and the card's time triples). `run-tuned-1080.bat` is kept only as a counter-example |
| Output at 2560x1440 | the copy-back is cheapest but preparing hits 46.6 ms and once 174.6 ms: a pathological fractional-scale path. Do not use |
| Dropping `VICTORY_WC_NO_BACK_MIN_KB` to 16 or 8 KiB to save 720p | no: 17.6 ms of copying back at 8 KiB. At 720p those ranges are not left on the card at all |
| PC FSR 4.1.2, `performance` (1920x1080 -> 3840x2160) | works, -0.3 ms (43.8 -> 43.5): the frame is the title's CPU work, the card was never the limit. A picture choice, not a speed one |
| [PR #443](https://github.com/Belobug/VIctory-Belobug-AMD/pull/443) exact mappings and lock waits | no gain on top of #477: it fixes waits that were on the copy-back path #477 removed |
| [PR #649](https://github.com/Belobug/VIctory-Belobug-AMD/pull/649) AVX2 codegen (`x86-64-v3`) | no change in three bracketed blocks (43.0 / 45.4 / 46.9) |
| The descriptor heap mirror (`HostHeap::sync_slots`) | 13.0 ms of work a frame, but only 0.66 ms of it is waited for: the rest hides behind preparing |
| `VICTORY_GPU_TIMING=1`, `LAND_THREADS=8`, `CARD_BUFFERS=1`, `JOIN_PARTS=16`, `THIN=0`, `CARD_AWAKE=0`, `MAP_THREADS`, all five ports off | no gain, or not decidable; see the doc |

## How to run it

You need Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with shader
model 6.6 and mesh shaders, and your own copy of the title. This tree is played
from the three launchers in its root; all of them keep the shader and image
caches in `D:\victory-data\belobug` (see `victory.toml`) so that the other
victory-family forks on this PC do not share them.

| Launcher | What it is |
|---|---|
| `run-tuned.bat` | **the one to play.** 3840x2160 output, `WC_NO_BACK`, the five ports, 8 GiB texture budget, the title's own upscaler. Log: `runs\memtrim.log` |
| `run-tuned-fsr.bat` | the same with PC FSR 4.1.2 at `quality` (2560x1440 -> 3840x2160): the same frame, a better upscaler. Needs the provider folder below. Log: `runs\memtrim-fsr.log` |
| `run-tuned-1080.bat` | 1920x1080 output. **Measured 2026-11-28: not worth using** with `WC_NO_BACK`; kept as a counter-example |

For PC FSR 4.1.2, put AMD's signed provider in one folder and point
`VICTORY_FIDELITYFX_DIR` at it. The whole `Kits/FidelityFX/signedbin` set has to
be there - upscaler, loader, framegeneration, denoiser, radiancecache - or the
layer fails to load it with `0xC000007B`. The real
`amd_fidelityfx_upscaler_dx12.dll` for SDK 2.4.0 is 57,664,272 bytes (blob
`6d01d9f5`); a smaller file from a mirror is another build and loads nothing.
The title profile and the PC FidelityFX bridge were ported from
[DexNom/VIctory](https://github.com/DexNom/VIctory/commit/ab34a8b21477c94da2a6cf07249138e8bc84602e).

`VICTORY_FSR_4K` takes `quality` (2560x1440), `balanced` (2258x1270) or
`performance` (1920x1080); all three present at 3840x2160. AMD's SDK offers FSR
4.1.2 for some cards only; where it does not, the newest 3.1 it offers is used
and the log says so, and where it offers neither the run stops before the title
starts and names what it was offered.

On a hybrid-graphics PC the runtime prefers a high-performance adapter with
Shader Model 6.6 and logs its name, vendor and device IDs. Set
`VICTORY_D3D12_ADAPTER` to a DXGI index or part of the adapter's name to choose.

## Building

```powershell
copy victory.example.toml victory.toml      # then set your paths, outside this folder
cargo build --locked --profile diag -p victory --features diagnostics
cargo xtask policy                          # no game-derived file in the tree
```

`diag` is the profile every measurement here is of: release code generation
plus the `diagnostics` feature, and light device removal (DRED) diagnostics by
default. It is not built with LTO (`lto = false`, 16 codegen units) so that a
rebuild is 40 s rather than minutes. Deploying means copying
`target\diag\victory.exe` over the one the launcher uses; stop the title first,
`taskkill /F /IM victory.exe`, and retry until it is gone - the title is loaded
inside the victory process, there is no separate game process.

## Known problems

- **Under `VICTORY_RELAY=2`, a descriptor slot can be rewritten while a list
  still in the air reads it**: black stripes across the screen and the faces of
  characters close to the camera disappearing. This is
  [#545](https://github.com/Belobug/VIctory-Belobug-AMD/issues/545), reproduced
  on RDNA3 and RDNA4. `VICTORY_LATE_WAIT=0` removes it and takes the frame from
  45.7 to 113.9 ms. [PR #636](https://github.com/Belobug/VIctory-Belobug-AMD/pull/636)
  makes a slot flip only at a moment with nothing in the air; it is built into a
  separate tree here (`D:\victory-belobug\VIctory-636`, `run-636.bat`) to be
  judged on the picture, because one measured block (43.0 and 53.2 ms, the
  second a stall) says nothing.
- **`WC_NO_BACK`'s own census** (13 minutes, 0 reads of a title page): about 20%
  of the ranges cannot be guarded, a write followed by a read inside the same
  second is not seen, and one tester in five saw a shirt turn dark grey once.
- Trees, signs and far objects flicker in some places; lighting is too bright or
  too dark after walking indoors until the title has run a while; textures of
  what just came into view appear late while the title streams; the first run of
  a new build stutters while the driver makes pipelines.
- Recording a frame in the world makes the title's own watchdog end it.

## Where the numbers are

[`docs/tuned-2026-11-27.md`](docs/tuned-2026-11-27.md) is the log of this fork's
tuning: how the benchmark works and what it can resolve (a same-session A/B,
heavy frames only, medians over 45 s of a fixed replay), the copy-back chapter
from 18.1 ms to 1.3 ms, the resolution lines that lost, FSR, AVX2, PR #443 and
#636, and the external forks whose issue trackers were read for candidates.

This tree stands on [Marvisak/VIctory](https://github.com/Marvisak/VIctory), its
forks (fxceless, avatarmanz's 7900XTX work, whose notes are kept in
[docs/README-fxceless.md](docs/README-fxceless.md)) and the PRs cited above.
VIctory source is GPL-3.0-or-later; the BCPack format tables are separate data
outside that license and are not in this repository.
