# btop-web - Memory footprint lab

Status: DRAFT for owner review, 2026-10-29.

## 1. Overview

The server's own held data is 1.07 MB. At steady state with one viewer the process holds 46,808 kB RSS: 20,824 kB (44.5%) is the NVIDIA driver that the first GPU sweep loads, 12,092 kB (25.8%) is glibc arena memory that the allocator keeps, and 10,748 kB (23.0%) is file-backed binary and library pages. The rest is thread stacks, the main heap, and the held data.

Chart sample history is 96,144 B, 0.20% of RSS and 0.62% of the server's own anonymous memory (15,108 kB). Storing it as 16-bit samples scaled to each chart's range would save 70,848 B, 0.07 MB and 0.15% of RSS. It is not worth doing.

One change pays: capping glibc arenas with `MALLOC_ARENA_MAX=2` saved 4,868 kB of RSS (11.3%) in a 60 s run with ten clients, and it needs no Rust change.

Without a viewer the sampler parks: the process holds 7,040 kB and the driver is not loaded. The first viewer adds 29,728 kB, mostly the driver and the first arenas. Each further viewer adds about 992 kB at ten clients, and the memory stays after the viewers leave because glibc keeps the arenas.

## 2. Method

- Binary: `tmp/live/btop-web-owner`, sha256 `87d7dcf4ff37eb2656a1918b9a698d13492c05f5ba5e842dac1666908c103853`, 5,527,080 B, built at 08:56. Another batch edited `app.js` at 09:30 and `app.css` at 09:38 after that build, so the embedded asset sizes are read from the source tree
- Host: 64 cores, three NVIDIA GPUs, 367 to 402 processes during the run because other sessions spawned and exited
- Main run: free loopback port 49815, default 250 ms tick, 1000 ms process refresh, read-only guard on. Phases: startup, sleep with no client, one client, ten clients, disconnected, then one client for 600 s. `/proc/<pid>/status`, `smaps_rollup`, `maps` and `smaps` were sampled throughout
- Arena check: the same binary started twice on free loopback ports, ten plain `curl -N` SSE readers each for 60 s, the second run with `MALLOC_ARENA_MAX=2`. A mapping is an arena when it starts on a 64 MiB boundary, a thread stack when it is a 2 MiB anonymous mapping, and driver memory when it or the mapping before it names `libnvidia-ml`, `libcuda` or `nv_dispwi`
- Frame dump: a separate 90 s run on a free port captured the five rendered fragments and the process row markup
- Model: the `layout_probe.rs` field sizes, the dumped HTML, and the source constants (`WINDOW_MS` 240,000, `PROC_WINDOW_MS` 20,000, 960 and 240 slots)
- Artifacts: `tmp/workflows/btop-web-mem-lab/checkpoints/measure.json`, `raw.json`, `arena.json`, `phases/frame_late/`
- Units: kB is 1,024 B; MB in section 5 is 1,048,576 B

## 3. Phases

| Phase | Clients | End, s | RSS, kB | Anonymous, kB | Threads | FDs |
|---|---:|---:|---:|---:|---:|---:|
| startup | 0 | 20 | 7,040 | 1,640 | 65 | 10 |
| sleep, no client | 0 | 80 | 7,040 | 1,640 | 65 | 10 |
| one client | 1 | 140 | 36,768 | 26,020 | 68 | 19 |
| ten clients | 10 | 200 | 45,692 | 34,944 | 68 | 28 |
| disconnected | 0 | 320 | 45,740 | 34,864 | 66 | 18 |
| steady, one client | 1 | 920 | 46,808 | 35,932 | 68 | 19 |
| viewers stopped | 0 | 940 | 47,208 | 36,332 | 66 | 18 |

The peak is 47,208 kB at 10 s after the steady viewer stopped. With no client the sampler is parked and the anonymous footprint is 1,640 kB, so every megabyte above that is viewer-driven.

The ten-client phase costs 8,924 kB over one client, 992 kB per extra viewer. After the clients disconnect the process keeps 45,740 kB: glibc retains the arenas, so the high-water mark is set by the largest viewer count, not by the steady data. The 600 s steady phase adds 1,068 kB over the disconnected value, about 0.1 MB per minute, and flattens.

## 4. Consumers

| Consumer | Size | Share of RSS | Basis |
|---|---:|---:|---|
| NVIDIA driver, anonymous | 20,824 kB | 44.5% | Measured: 18,868 kB mapping after `libnvidia-ml.so.1`, 1,524 kB after `libcuda.so.1.1`, plus smaller adjacent mappings; owner-instance cross-check 22,440 kB |
| glibc malloc arenas | 12,092 kB | 25.8% | Measured: eleven 64 MiB-aligned anonymous mappings in the steady-state dump |
| Thread stacks (tokio workers, main, blocking pool) | 1,000 kB resident | 2.1% | Measured: 67 anonymous 2 MiB mappings for 68 threads; 64 are tokio workers, the rest are the main thread and blocking-pool threads; 136 MiB of virtual address space |
| Main heap | 532 kB | 1.1% | Measured `[heap]` |
| Process snapshot | 471,446 B | 1.0% | Modelled from 402 rows and the layout probe |
| Published frames | 467,374 B peak | 1.0% | Measured means: 233,687 B per frame, two frames at publish |
| Chart path strings | about 92 kB | 0.2% | Measured path bytes, four live copies |
| Chart sample histories | 96,144 B | 0.2% | Modelled from the window constants and series counts |
| Theme CSS | 34,377 B | 0.07% | Measured index page |
| Identity caches | under 16 kB | under 0.1% | Modelled: passwd map, cgroup members, kernel pid set, CPU name, sensor, battery and holder caches |
| Other snapshot structs | about 8 kB per frame | under 0.1% | Modelled: `CpuSnapshot` 352 B, 64 `CoreUsage` of 56 B, 64 core temperatures, and the small memory, network and GPU structs |
| Other anonymous mappings | about 324 kB | 0.7% | Remainder after the rows above |
| Binary and shared libraries, file-backed | 10,748 kB | 23.0% | Measured shared clean RSS; the 246,864 B of embedded assets sit in this file-backed binary mapping and are not anonymous |

### 4.1 Process snapshot

The 402-row snapshot is 471,446 B, the largest single piece of server data.

- Cache: 402 `CachedProcess` entries of 528 B, 212,256 B. Each is `ProcEntry` 320 B plus `cpu_t` 8, `cpu_s` 8, `starttime` 16, `RowGraph` 88 and `RowGraph` 88. This cache holds the pid identity and the per-row graph state, so a pass detects pid reuse by comparing `starttime` instead of rereading the identity for every row
- Two live row vectors: 402 rows x 320 B x 2, 257,280 B. The slow pass keeps one `ProcSnapshot` and the published frame keeps another
- Row graph path strings: 6 own graphs and 3 sum graphs, 955 B, in two copies, 1,910 B
- Row histories: nine histories of 20 samples, 3,024 B, counted under chart histories

The per-row name and command are `Arc<str>` shared between the cache and every row copy, so a tick copies no string.

### 4.2 Frames

The sampler renders all five fragments every tick and publishes them as one `Arc<Frame>`. Mean sizes over the 600 s steady run:

| Fragment | Bytes | Share of frame |
|---|---:|---:|
| proc | 147,319 | 63.0% |
| cpu | 28,207 | 12.1% |
| gpu | 26,301 | 11.3% |
| net | 18,779 | 8.0% |
| mem | 13,081 | 5.6% |
| total | 233,687 | 100% |

Two frames exist while a new one is built and the old one is still held by the channel or a stream, 467,374 B. The held value between ticks is one frame, 233,687 B. The proc fragment is rendered four times per second although a stream sends it only when the slow sequence changes.

### 4.3 Chart histories

Eleven full-window series hold 960 samples each: CPU total, memory used, network down and up on three interfaces, and three GPUs. At 8 B per sample the payload is 84,480 B; with 1,024-slot `VecDeque`s and 80 B `History` structs the allocation is 90,992 B.

The process count history spans 240 samples at the 1 s slow cadence: 1,920 B payload, 2,128 B with a 256-slot deque and the struct.

The row sparklines span 20 samples at the slow cadence: nine histories, 1,440 B payload, 3,024 B allocation.

Total: 87,840 B payload, 96,144 B allocation. Each `History` also keeps the smoothed 1 s EMA as one `f64` scalar.

### 4.4 Chart path strings

Each panel chart holds a line and an area `String`; the slow pass clones them and each rendered frame copies them into its HTML. Per copy: CPU 3,225 B, memory 3,173 B, network 6,158 B, GPU 9,589 B, the process count 355 B and the nine row graphs 955 B, about 23.5 kB. The snapshot, the slow pass and one copy per frame make about 92 kB across two frames.

### 4.5 Identity caches

The passwd map is 1,321 B of text parsed into a `BTreeMap` of 26 users. The cgroup member list holds about 400 pids. The kernel pid set, CPU model name, sensor set, battery set and GPU holder set are each under 2 kB. The per-pid identity (pid, `starttime`, cpu baselines) is inside the 528 B process cache entry above. All identity caches together are under 16 kB.

### 4.6 Embedded assets

The binary embeds 246,864 B of assets: `app.js` 76,510, `app.css` 62,404, `htmx.min.js` 52,182, the GalaxaTron face 33,480, `htmx-sse.min.js` 8,896, `theme.js` 6,072, the licence and derivation notes 6,804, and the icon 516. They are file-backed rodata inside the 5,527,080 B binary, shared between processes and evictable, not heap.

## 5. The 16-bit chart sample answer

The owner's question: what fraction of the footprint is chart sample history, and what would storing it as 16-bit scaled to each chart's range save?

| Denominator | Bytes | Chart sample share |
|---|---:|---:|
| RSS | 47,931,392 | 0.20% |
| Anonymous | 36,794,368 | 0.26% |
| Server's own anonymous memory | 15,470,592 | 0.62% |
| Modelled held data | 1,069,341 | 9.0% |

At 2 B per sample in the same slot counts: 11 full-window histories x (1,024 slots x 2 B + 80 B) = 23,408 B, the count history 256 x 2 + 80 = 592 B, and nine row histories x (32 x 2 + 80) = 1,296 B, 25,296 B in total. The saving is 70,848 B of allocation, 65,880 B of payload: 0.07 MB, 0.15% of RSS, 0.46% of the server's own anonymous memory.

The precision is enough: 65,536 steps over a 0 to 100 range is 0.0015 percentage points, and the plot is 77.5 px tall. The obstacle is the code. A u16 ring needs a stored extent per history, a conversion on every read in `chart`, `line` and `knots`, and a rescale or a raw extent when an auto-scaled ceiling changes. The bucket peak in `knots` compares stored values, so quantisation can change which sample is chosen as a peak. The 1 s EMA stays f64 and the gauge keeps reading it. That is a change to the chart core, the histories and their tests.

The answer is no. Chart sample history is 0.20% of RSS, and 16-bit storage saves 0.15% of RSS. The measured arena saving is about 69 times larger.

## 6. Ranked reducible consumers

The order weighs the saving against the change and its risk. Saving is against the 46,808 kB steady-state RSS unless stated.

| Rank | Consumer | Saving | Risk | Change and verdict |
|---:|---|---:|---|---|
| 1 | glibc arenas | 4,868 kB measured, 11.3% | Low | Set `MALLOC_ARENA_MAX=2` in the container environment. No Rust change; the test ran 60 s with ten clients and the server stayed up. Do it, then re-measure on the deployed image |
| 2 | Proc fragment re-render | 147,319 B peak, 0.31%, plus 3 of 4 renders per second | Low | Keep the last rendered proc string in the sampler and reuse it while `slow_seq` is unchanged; change the `Frame` event strings to `Arc<str>`. Do it when the frame code is next touched |
| 3 | Process snapshot second copy | 128,640 B, 0.27% | Medium | Make `Snapshot.proc` and `SlowPass.proc` an `Arc<ProcSnapshot>` and share it instead of cloning. Do it inside a future process change, not alone |
| 4 | Thread stacks | about 840 kB, 1.8% | Medium | Set `worker_threads` from 64 to 8 in a tokio `Builder`. Reject: the saving exists only on many-core hosts and it trades throughput for 1.8% |
| 5 | 16-bit chart samples | 70,848 B, 0.15% | Medium to high | A u16 ring with a stored extent in `chart.rs`, conversions in `chart`, `line` and `knots`, and new tests. Reject: 0.15% of RSS does not pay for the chart core |
| 6 | Chart path strings | about 92 kB | High | No reduction without moving the charts client-side, which the no-JSON-API rule forbids. Reject |
| 7 | Theme CSS | 34,377 B | Low | One string rendered at startup and used by every page. Reject |
| 8 | Embedded assets | 246,864 B file-backed | Low | Removing `htmx.min.js` or the font changes features; the pages are shared and evictable. Reject |
| 9 | Identity caches | under 16 kB | High | The per-row identity detects pid reuse; the small caches are already read once and kept. Reject |
| 10 | Process cache per-row structs | 212,256 B | High | The 528 B entry carries btop's fields and the row graph state. Trimming breaks collection parity or the graphs. Reject |

The arena test ran at ten clients, where the default arena footprint was 8,520 kB and the capped footprint 2,484 kB. The steady state keeps 12,092 kB in arenas, so 4,868 kB is the measured saving at that load, not the steady-state ceiling. The idle process is already minimal: the sampler parks with no viewer at 7,040 kB and never loads the driver.

### 6.1 Rejected for unsafe or a new dependency

- `mallopt` through libc needs `unsafe`, which `unsafe_code = "forbid"` bans
- `std::env::set_var` at startup is `unsafe` in edition 2024, so the arena cap must come from the environment the container sets, not from Rust
- A global allocator crate (mimalloc or jemalloc) is a new native dependency. It targets the same arena retention that `MALLOC_ARENA_MAX` addresses at no dependency cost. Consider it only if the environment setting fails to deliver after a deployment measurement, and state the measured delta first

### 6.2 What cannot be reduced

- NVIDIA driver, 20,824 kB (44.5%). NVML and libcuda load on the first GPU sweep and stay for the process lifetime. Removing it means removing the GPU panel's data, so this is a feature decision, not a memory fix. The idle process never loads it
- File-backed binary and libraries, 10,748 kB (23.0%). Shared, evictable, and smaller only by dropping a vendored library or the font
- The server's held data, 1.07 MB (2.3%). This is the working set of the dashboard and is already stored once or twice per frame

## 7. Limitations

- The 64-core host drives the arena count and the 65 to 68 threads. A container with 2 or 4 cores creates fewer workers and fewer arenas, so the absolute arena saving does not transfer unchanged; the mechanism does
- The process count moved between 367 and 402 during the run because other sessions spawned and exited. The process snapshot uses 402 rows, the largest count measured
- Driver memory is attributed by mapping adjacency to the driver libraries, not by the driver's own accounting. The owner-instance cross-check puts it at 22,440 kB, 1,616 kB above the main run
- The asset bytes are read from the source tree after the binary was built, as noted in section 2
- The frame sizes are means over the steady run. The largest proc event observed in the main run was 165,701 B, so the two-frame peak is a mean, not the maximum
