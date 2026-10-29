# Defects

Defects of btop-web, each with its reproduction and root cause. `pm-tools` is the only way into this file.

## Authors

- `@kj` Konrad Jelen

## GPU collection `GPU`

GPU telemetry from NVML, ROCm SMI and the supported fields

- log: 2026-10-25T03:14:07Z @kj category created

- [x] `DEF-GPU-1-AMD-ROCM` **AMD GPU reads DRM sysfs, not ROCm SMI** - MAJOR; btop 1.3.0 reads AMD through librocm_smi64.so for name, power cap, busy and memory percent, clocks, power, VRAM and PCIe; btop-web reads /sys/class/drm and synthesizes a name. This host has no AMD GPU, so the gap is untested live
  - evidence: AMD path reads ROCm SMI behind an optional feature: RsmiBackend seam with 5 hardware-free tests, cfg(feature=rocm) impl over rocm_smi_lib 0.3.2; make ci 110 tests; real-crate build untested here (no librocm_smi64)
  - repro: on an AMD host with ROCm SMI, compare btop's AMD GPU panel with btop-web's; the sources differ
  - log: 2026-10-25T03:14:07Z @kj added
  - log: 2026-10-25T13:49:42Z @kj user decision 2026-10-25: implement the ROCm SMI path now, with parsing fixtures only, no AMD hardware on this host
  - log: 2026-10-25T15:15:43Z @kj closed
- [x] `DEF-GPU-12-FEATURES` **NVML supported flags are re-probed** - MINOR; btop caches the supported-function flags at init and keeps the last sample; btop-web retries every field each sample and renders unavailable after a failure
  - evidence: supported flags cached at init and a failed read keeps the last sample; supported_flags_start_on_and_a_failed_handle_turns_them_off and a_failed_read_keeps_the_last_sample pass
  - repro: make an NVML field call fail after init; btop keeps the value, btop-web shows unavailable
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T03:49:02Z @kj closed
- [x] `DEF-GPU-40-CONTAINER-HIDDEN` **GPUs hidden in the container** - MAJOR; in container mode the GPU panel reads unavailable, but the container has the 3 GPUs (nvidia-smi in it lists RTX PRO 4000, 6000 Max-Q, 5000 72GB); the devices NVML lists inside the container are the container's devices and must be shown
  - evidence: container mode lists the 3 GPUs nvidia-smi shows, values equal, load graph drawn; make ci 173 green; binary 44a72c34; tmp/workflows/btop-web-container-devices/checkpoints/final-2.json
  - related: ACC-SCOPE-64-CONTAINER
  - repro: port 40290 build 7fec8c28 in this Docker container: GPU panel header 'unavailable'
  - test-tags: UNIT, FUNCTIONAL
  - root-cause: 2026-10-27T18:29:11Z @kj ACC-SCOPE-64 spec withheld every NVML value because NVML reports device-wide usage; it did not treat the devices passed into the container as the container's own
  - log: 2026-10-27T18:29:11Z @kj added
  - log: 2026-10-27T19:30:32Z @kj closed

## Network collection `NET`

Interface enumeration, rates, totals and graph scaling from btop's Net::collect

- log: 2026-10-25T03:14:07Z @kj category created

- [x] `DEF-NET-2-AGGREGATE` **Net panel shows a cross-interface rate sum** - MEDIUM; btop has no cross-interface rate field; its Total is the selected interface's cumulative bytes, which btop-web does not render. NetInterface.total_down and total_up already hold the right values
  - evidence: the net header renders the selected interface's top_down and total_down from NetInterface, not a cross-interface sum; the live page shows eth0 top and total scoped to that interface
  - repro: open the net panel beside btop; compare Total down/up with btop's selected-interface Total
  - log: 2026-10-25T03:14:07Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `DEF-NET-3-AUTOSCALE` **Net graphs do not auto-scale** - MEDIUM; with net_auto=true btop computes graph_max, max_count and rescale from the last five samples; NetSnapshot has no graph_max field and the web graph stays on a fixed scale
  - evidence: net charts are built from the selected interface histories with graph_max_down and graph_max_up as the ceiling; the auto-scale unit tests pass; live SSE frames carry the down and up paths
  - repro: set net_auto=true, generate a burst and compare the graph scale with btop's
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T03:49:08Z @kj collector auto-scale landed in net.rs with graph_max_down/graph_max_up; net.html renders no graph yet, UI half remains
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `DEF-NET-42-LOOPBACK-PICK` **Network panel shows lo** - MEDIUM; after the 18:28 host restart the Network panel picked lo, so download equals upload (303 KiB/s each) and shows proxy traffic, not eth0
  - evidence: without net_iface the panel shows eth0 live and on a fixture where lo is busier; lo only when alone; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - repro: fresh start with lo total above eth0: /proc/net/dev lo 2.42 GB vs eth0 0.28 GB; header reads 'lo'
  - test-tags: UNIT
  - root-cause: 2026-10-27T18:29:11Z @kj btop's rule (btop_collect.cpp Net::collect) picks the interface with the largest download + upload total; lo carries the JupyterLab proxy traffic
  - log: 2026-10-27T18:29:11Z @kj added
  - log: 2026-10-27T19:40:40Z @kj owner decided 2026-10-27: skip lo; pick the busiest interface other than lo when one is up, lo only when nothing else exists; net_iface in the config still forces any interface
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint

## Configuration `CONF`

btop.conf keys, defaults, precedence and validation

- log: 2026-10-25T03:14:08Z @kj category created

- [x] `DEF-CONF-4-KEYS` **Collector config keys are not read** - MEDIUM; only the compiled btop 1.3.0 defaults apply; the cpu, gpu, net and proc display keys in btop.conf have no effect
  - evidence: 27 keys in config.rs and btop-web.conf with btop defaults and validation; invalid values stop startup with file:line; config_keys_change_btop_behaviour and collector_keys_are_read_from_the_file pass; make ci 85 tests green
  - repro: run btop with this host's btop.conf (proc_sorting=cpu direct, net_auto=False) beside btop-web; process order and graph scale differ
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T03:49:01Z @kj closed

## CI gates `GATE`

The local Makefile gates and the GitHub workflow that repeat them

- log: 2026-10-25T03:14:08Z @kj category created

- [x] `DEF-GATE-5-DENY` **make deny skips when cargo-deny is absent** - MEDIUM; the target prints a skip and exits 0, so deny.toml is never evaluated locally; GitHub CI runs cargo-deny-action
  - evidence: cargo-deny 0.20.2 installed; make deny runs cargo deny check with no skip line, exit 2 on a real advisory; bans, licenses and sources evaluate; skip branch kept for a host without the tool
  - repro: run make deny on this host; it prints a skip and exits 0
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T13:49:42Z @kj user decision 2026-10-25: install cargo-deny on this host so make deny evaluates deny.toml; the skip stays a guard for hosts without the tool
  - log: 2026-10-25T14:01:48Z @kj closed
- [x] `DEF-GATE-23-ADVISORY` **cargo-deny fails on an unmaintained build dependency** - MAJOR; make deny exits 2: basic-toml 0.1.10 is unmaintained, RUSTSEC-2026-0374, through askama_derive 0.16.1; askama's newest release is 0.16.1, the version in use, so no upgrade drops it; the GitHub CI Deny step runs the same file and will fail
  - evidence: RUSTSEC-2026-0374 ignored in deny.toml:4-6 with the build-time-only reason; make deny exits 0 with advisories ok, bans ok, licenses ok, sources ok; no other rule weakened
  - repro: make deny on a host with cargo-deny installed
  - root-cause: 2026-10-25T14:01:48Z @kj basic-toml is a build-time dependency of askama's derive macro, not shipped at runtime; no askama release replaces it
  - log: 2026-10-25T14:01:48Z @kj added
  - log: 2026-10-25T15:15:43Z @kj closed
- [x] `DEF-GATE-28-HELP-DOCKER` **make help omits the docker target** - MINOR; make help prints 16 of 17 targets and leaves out docker; README.md lines 53 and 143 say it lists every target; likely cause: the shortcut comment at Makefile:59 sits between the ## line and docker:, and the help parser only matches a ## line directly above a target
  - evidence: shortcut comment moved above the ## line; make help prints 17 targets equal to the Makefile set, docker included; wf_46141fe3-f77 tmp/workflows/btop-web-gpu-load/checkpoints/final-1.json check 5
  - repro: make help | grep -c docker prints 0
  - test-tags: MANUAL
  - log: 2026-10-26T17:08:11Z @kj added
  - log: 2026-10-26T18:59:43Z @kj closed
- [x] `DEF-GATE-34-TOOLS-OPENSSL` **make tools fails on cargo-outdated** - MINOR; make tools exits 2 on this host: cargo install cargo-outdated builds openssl-sys 0.9.113, which needs pkg-config and the OpenSSL development headers; the four tools before it install; no Makefile target runs cargo-outdated, only tools and doctor list it
  - evidence: cargo-outdated (used by no target) removed from the tools and doctor lists and the README row; make tools, make doctor and make help exit 0; make ci 145 tests; wf_b8673c4e-6a8 tmp/workflows/btop-web-gate-tools/checkpoints/verify.json, patch tmp/workflows/btop-web-round1-fix/patches/gate-1.patch
  - repro: make tools; see logs/btop-web-round1-fix-make-tools.log
  - test-tags: MANUAL
  - log: 2026-10-27T10:24:14Z @kj added
  - log: 2026-10-27T10:34:46Z @kj closed

## CPU collection `CPU`

CPU usage, frequency, sensors and battery from btop's Cpu::collect

- log: 2026-10-25T03:14:08Z @kj category created

- [x] `DEF-CPU-6-FIRST-SAMPLE` **First CPU frame is the since-boot average** - MINOR; btop seeds Cpu::collect at init and never draws the since-boot value; btop-web publishes it as its first frame
  - evidence: CpuTimes.seeded publishes no first percentage; first_sample_seeds_the_delta_window and tests/e2e.rs pass; verify shows the first CPU frame unavailable and the next 2 s frame the first delta
  - repro: start btop-web fresh with update_ms 2000 and read the first /events cpu frame; it is the /proc/stat since-boot average
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T03:49:02Z @kj closed
- [x] `DEF-CPU-10-BATTERY` **Battery Auto selection differs** - MINOR; Auto picks the first sorted Battery then UPS; btop iterates an unordered_map, so a host with several batteries can choose a different device
  - evidence: Auto keeps /sys/class/power_supply order; selected_battery selects then falls back; Auto BAT0 74.0% vs UPS0 10.0%; config_keys_change_btop_behaviour passes
  - repro: on a host with two batteries and selected_battery=Auto, compare the chosen device and percentage with btop
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T03:49:02Z @kj closed
- [x] `DEF-CPU-41-CONTAINER-CORES` **Cores hidden in the container** - MAJOR; in container mode the Cores panel reads unavailable, but the container's cpuset.cpus.effective is 0-63; the cores in the cpuset are the container's cores and must be shown with their per-core usage
  - evidence: container mode shows one core row per cpuset CPU: 64 rows for 0-63 live, 4 for the 0-3 fixture; per-core values track /proc/stat; make ci 173 green; tmp/workflows/btop-web-container-devices/checkpoints/final-2.json
  - related: ACC-SCOPE-64-CONTAINER
  - repro: port 40290 build 7fec8c28: Cores panel header 'unavailable'
  - test-tags: UNIT, FUNCTIONAL
  - root-cause: 2026-10-27T18:29:11Z @kj ACC-SCOPE-64 spec withheld per-core usage because /proc/stat counts every process on a core; it did not treat the cpuset's cores as the container's own
  - log: 2026-10-27T18:29:11Z @kj added
  - log: 2026-10-27T19:30:32Z @kj closed
- [x] `DEF-CPU-44-CONTAINER-LOAD` **Load per core unavailable in the container** - MAJOR; in container mode the CPU panel reads 'load per core unavailable' and the CPU footer 'load unavailable'; the owner needs the load shown; it must be the container's own load, not the VM's
  - evidence: container load from the kernel calc_load over cgroup R and D threads every 15 s, never unavailable or the VM's value, rises with 4 busy threads; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: ACC-SCOPE-64-CONTAINER
  - repro: port 40290 build 44a72c34 in this Docker container: CPU panel 'load per core unavailable'
  - test-tags: UNIT, FUNCTIONAL
  - root-cause: 2026-10-27T19:51:21Z @kj ACC-SCOPE-64 spec withheld /proc/loadavg because the kernel averages every task on the VM (4,799 tasks, load 5.96) while the container has 2,528; no container-scoped load was computed
  - log: 2026-10-27T19:51:21Z @kj added
  - log: 2026-10-27T23:48:02Z @kj owner 2026-10-28: keep the exact kernel-method container load at about 2.6 ms CPU per update (+26%, about 0.65% of one core); run wf_72f6efec-601 gate 2 bound 0.2 waived as a sampling race (rise and fall pass), merged by btop-web-merge-1
  - log: 2026-10-27T23:48:23Z @kj owner 2026-10-28: reduce the sampling frequency to cut the cost; count every 15 s with decay factors exp(-15/60), exp(-15/300), exp(-15/900) = 1595, 1948, 2014 over 2048 (the same rule gives the kernel's 1884, 2014, 2037 at 5 s); change d-44-freq in btop-web-merge-1
  - log: 2026-10-28T01:20:23Z @kj merge-1 verify round 1: A/A control shows two same-instant sweeps differ by up to 3.0 in the folded 1 min value in this busy container; within-2.0 bound dropped for round 2; gate keeps never unavailable, never the VM's value, rise of page minus background at least 1, fall
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint

## Dashboard display `VIEW`

What the panels render and how an unavailable value is drawn

- log: 2026-10-25T03:14:08Z @kj category created

- [x] `DEF-VIEW-7-CPU-NAME` **CPU model name is collected but not rendered** - MINOR; CpuSnapshot.name holds btop's CPU box title, but cpu.html has no name row
  - evidence: cpu.html renders CpuSnapshot.name in a titled .cpu-name row and no row when None; cpu_name_row_renders_only_when_collected passes; the live fixture page shows Ryzen Threadripper PRO; make ci 92 tests green
  - repro: open the CPU panel; no model name row is present
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T07:12:46Z @kj closed
- [x] `DEF-VIEW-8-UNAVAILABLE-BAR` **An unavailable meter draws an empty bar** - MINOR; bar(None) is 0, so the RAM and disk meters draw an empty bar while the text correctly reads unavailable; no numeric zero is shown
  - evidence: bar() returns Option<u8>; None emits data-state=unavailable with no --value on cpu, memory, swap, disk and gpu meters, drawn as a hatched track; the empty-fixture http test finds three unavailable meters and no 0%; live pages agree
  - repro: run with BTOP_WEB_PROC_ROOT=crates/btop-web/tests/fixtures/cpu/unavailable/proc and open the dashboard
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T07:12:46Z @kj closed
- [x] `DEF-VIEW-16-ROUNDED-CHIP` **Indicators are rounded pills, not house chips** - MAJOR; READ-ONLY and every status indicator use the house chip of the JupyterLab device-bridge extension: compact, bordered, square or hairline radius, uppercase label. The current design draws a rounded pill; rounded pills are banned everywhere, including badges, buttons and panels
  - evidence: max computed border-radius is 2 px across the live page; .chip uses a 1 px border, 2 px radius, uppercase and a 12% tint; no pill radius exists in the CSS
  - repro: open docs/design/index.html; the READ-ONLY badge is a rounded pill; compare it with the device-bridge chip in the jupyterlab-extension design system
  - test-tags: E2E, MANUAL
  - root-cause: 2026-10-25T03:54:50Z @kj design drew a generic dashboard badge instead of the house component
  - log: 2026-10-25T03:54:50Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `DEF-VIEW-17-PAGE-CONTROLS` **Theme and mode controls sit on the dashboard** - MAJOR; the dashboard body carries telemetry only. Theme pair, light and dark mode and every other setting live in a config menu opened from the page chrome, not in the header of the page itself
  - evidence: theme select with 5 options, mode buttons, GPU chips and pause exist only inside #config; the page body has no theme or mode control; the popover opens on click and closes on Escape
  - repro: open docs/design/index.html; the theme picker and the light/dark toggle sit in the header instead of a config menu
  - test-tags: E2E, MANUAL
  - root-cause: 2026-10-25T03:57:42Z @kj controls were added to the first header design and never moved out
  - log: 2026-10-25T03:57:42Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `DEF-VIEW-25-HUMAN-ROUNDING` **Displayed byte and rate values round where btop truncates** - MAJOR; human() in collector/mod.rs prints one decimal and rounds; btop 1.3.0 floating_humanizer prints three significant digits and truncates. Memory used, swap used, network rates and every GPU VRAM total therefore differ in the last displayed digit although the underlying bytes agree, so ACC-PARITY-5-MATCH is not met.
  - evidence: human() is an exact floating_humanizer port; identical to compiled C++ on 9358 values to 163.8 PiB; live memory and GPU strings equal both sides; make ci 111 tests
  - related: ACC-PARITY-5-MATCH
  - repro: python3 -I tmp/workflows/btop-web-parity6/verify/verify-live-rerun.py: three ticks, btop reads 167 GiB and btop-web reads 167.8-167.9 GiB for memory used; live.json records gpu0/1/2_total 0/14 exact each
  - test-tags: MANUAL
  - root-cause: 2026-10-25T16:22:42Z @kj collector/mod.rs human() formats {value:.1} with rounding; btop btop_tools.cpp:399 floating_humanizer truncates to three significant digits and names the base unit Byte
  - log: 2026-10-25T16:22:42Z @kj added
  - log: 2026-10-25T20:22:01Z @kj closed
- [x] `DEF-VIEW-26-MENU-DISCOVERY` **Config trigger is not findable** - MEDIUM; The config button sits inside the CPU panel header among the clock and LIVE chip on 05, and inside the CPU head on the variations; a reader looking for settings does not find it. Expected one visible config button in fixed page chrome, theme and mode only in its menu
  - evidence: All ten round-2 variations carry one CONFIG button in a fixed top utility bar at x 1379.41-1432, y 4-22 (verify gate 2/tidy2), opening the theme and mode popover; no theme control in any body. 05 keeps its accepted in-panel control as the reference
  - repro: open 05-grafana-panels.html or any 05.0x page and look for the theme selector without knowing the CONFIG chip is in the CPU header
  - test-tags: MANUAL
  - root-cause: 2026-10-25T20:37:23Z @kj the trigger is styled as one telemetry chip among many and its position varies per page
  - log: 2026-10-25T20:37:23Z @kj added
  - log: 2026-10-25T22:11:30Z @kj closed
- [x] `DEF-VIEW-27-PROXY-PATHS` **Page broken behind the lab proxy** - MAJOR; Opened through /user/konrad.jelen/proxy/40290/ the page loaded without CSS, scripts or live data
  - evidence: index.html refs relative; http test page_references_assets_and_events_relatively; make ci 120 tests; proxied Chrome render at 1440x900: 0 failed responses, live CPU value, no scroll; 6 assets 200, 15 SSE frames in 5 s
  - repro: start btop-web, open /user/<name>/proxy/40290/ in the lab, watch the page stay unstyled and static
  - test-tags: UNIT, E2E
  - root-cause: 2026-10-26T14:38:43Z @kj templates/index.html referenced /assets/... and sse-connect=/events absolutely, so the browser requested them at the lab root instead of the proxied app
  - log: 2026-10-26T14:38:43Z @kj added
  - log: 2026-10-26T14:38:43Z @kj closed
- [x] `DEF-VIEW-37-CORE-LABEL` **Core rows lack name, percent sign and bar colour** - MEDIUM; at the drop-p4 tier (width up to 1100 px or height up to 640 px) the cores panel hides every core name; each value prints as 50, not 50%; every bar is one flat colour instead of btop's cpu gradient; owner reported again 20:40
  - evidence: rows read C00-C63 with % at 88 of 88 sweep sizes, bars in btop's cpu gradient; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - related: ACC-VIEW-54-CORE-LABEL
  - related: ACC-VIEW-56-CORE-COLOUR
  - repro: open the page at 1024x768: each cores row shows meter and number only, no C0 label, no percent sign
  - test-tags: UNIT, FUNCTIONAL
  - root-cause: 2026-10-27T11:55:18Z @kj app.css drop-p4 rule hides .core-row .cl; cores.html prints the smoothed percent with no percent sign
  - log: 2026-10-27T11:55:18Z @kj added; reason: names the tier bounds, both faults and the owner's pass condition
  - log: 2026-10-27T19:42:15Z @kj owner screenshot 20:40: no core names, no %, flat bar colour; fix d-37 and acc-56 are in proc-tile run wf_8ca31f9f-9b8, building, not deployed
  - log: 2026-10-27T19:42:17Z @kj amended title "Core rows lose labels and the percent sign" -> "Core rows lack name, percent sign and bar colour"; text "at the drop-p4 tier (width up to 1100 px or height up to 640 px) the cores panel hides every C0-C63 label, so a row cannot be matched to its core; each value prints as 50, not 50%; owner asked 2026-10-27: every row keeps its label at every tier and shows the percent sign" -> "at the drop-p4 tier (width up to 1100 px or height up to 640 px) the cores panel hides every core name; each value prints as 50, not 50%; every bar is one flat colour instead of btop's cpu gradient; owner reported again 20:40"
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [-] `DEF-VIEW-43-STALE-RESTART` **Page stuck STALE after server restart** - MEDIUM; after port 40290 restarted at 14:27, the owner's open tab showed STALE since 14:28:10 while the server streamed 13-14 frames per event per 3 s directly and through the proxy; cause unknown, reproduce first
  - repro: open the page, restart the server, watch the live chip for 60 s without reload
  - test-tags: FUNCTIONAL
  - log: 2026-10-27T19:22:19Z @kj added
  - log: 2026-10-27T19:26:41Z @kj not reproduced on 7fec8c28: server stopped 4 s and restarted on the same port; chip live again after 1-6 s direct, 1 s via lab proxy, live for 94 s after; the 14:28:10 stale start matches a make build by the container-scope spec agent
  - log: 2026-10-28T09:41:51Z @kj rejected: not reproduced: after a server restart the page is live again in 1-6 s direct and 1 s through the lab proxy; owner agreed 2026-10-28 to refile if it recurs
- [x] `DEF-VIEW-45-STAT-FOOT` **Footer items move** - MEDIUM; the footer items of the Processes, CPU and Memory tiles (top, 80% cpu, 80% mem; frequency, load; used, available) shift left and right every update; owner screenshot paste-20261028-110553.png; fix: each item keeps a fixed position
  - evidence: 14 footer items over 5 s moved 0.0 px while 4 changed text; batch-2 final gate 2 and merge smoke; build 9c91f603 live on 40290
  - related: ACC-LAYOUT-74-RESERVED-WIDTH
  - repro: watch the Processes tile footer for 10 s at 1440x900; 80% cpu and 80% mem move as the top process name and values change
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T10:07:43Z @kj likely cause: .stat-foot in app.css is a flex row with justify-content: space-between, so each item's x depends on the widths of its neighbours
  - log: 2026-10-28T10:07:43Z @kj added
  - log: 2026-10-28T15:38:33Z @kj closed
- [x] `DEF-VIEW-46-LEGEND-SHIFT` **GPU legend text moves** - MEDIUM; in the GPU legend the VRAM text after the load % moves when the load changes between one, two and three digits (0%, 10%, 100%); owner screenshot paste-20261028-111021.png
  - evidence: GPU legend reserved widths; batch-2 verify-1 gate 1: name, load, VRAM, temperature and power all shown uncut at 1440x900 and 1024x768, no legend overflow
  - related: ACC-LAYOUT-74-RESERVED-WIDTH
  - repro: watch the GPU legend while a GPU load crosses 10%; the VRAM text shifts right
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T10:11:50Z @kj likely cause: .legend-value prints the % inline with no reserved width, so the VRAM span starts where the % text ends
  - log: 2026-10-28T10:11:50Z @kj added
  - log: 2026-10-28T15:38:33Z @kj closed
- [x] `DEF-VIEW-47-CELL-SHIFT` **Process table values move other text** - MEDIUM; in the process table, changing Cpu% and MemB values move other text; in the screenshot the Cpu% value sits about 3 px higher in rows that show the small CPU graph, and the graph's x follows the value width; owner screenshot paste-20261028-120438.png
  - evidence: process table values move no other text; batch-2 verify-1 gate 5: maximum movement 0.281 px over 391 sizes
  - related: ACC-LAYOUT-74-RESERVED-WIDTH
  - repro: watch the process table Cpu% and MemB cells for 20 s at 1440x900 while values change between 0.0 and two digits
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T11:06:09Z @kj cause unknown; reproduce first; table-layout is fixed, so the movement is likely inside the Cpu% cell (graph and value inline)
  - log: 2026-10-28T11:06:09Z @kj added
  - log: 2026-10-28T15:38:33Z @kj closed
- [x] `DEF-VIEW-48-GPU-FACT` **Processes tile shows gpu unavailable** - MEDIUM; in container mode the Processes tile always reads 'gpu unavailable'; a permanent unavailable fact breaks ACC-VIEW-23-RELEVANT; fix: count this container's processes that use a GPU, or omit the fact when it cannot be read; owner screenshot paste-20261028-120926.png item 1
  - evidence: gpu fact reads a number ('gpu 1'); batch-2 final gate 2; holder scan costs +12.3 percent CPU per tick inside the 9-18 percent band
  - repro: open the page in the container at 1440x900; Processes tile facts read 'gpu unavailable'
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T11:14:28Z @kj likely cause: the count comes from NVML process lists, whose pids are host pids that the container's /proc cannot map
  - log: 2026-10-28T11:14:28Z @kj added
  - log: 2026-10-28T11:20:53Z @kj owner chose 2026-10-28: count this container's processes holding a GPU device (/dev/nvidia0-N) open, without btop-web itself; sample less often if the cost is measurable
  - log: 2026-10-28T11:44:20Z @kj owner 2026-10-28: count holders of every GPU node that applies (/dev/nvidiaN, WSL2 /dev/dxg, AMD /dev/kfd and /dev/dri/renderD*); prefer maintained libraries for GPU access; unreadable processes give a lower bound 'n+'
  - log: 2026-10-28T15:38:33Z @kj closed
- [x] `DEF-VIEW-49-FILTER-STYLE` **Process filter box looks unstyled** - MINOR; the process filter input looks like a bare wireframe box with no house style; it should match the JupyterLab input and house chip style in all themes, dark and light; owner screenshot paste-20261028-120926.png item 2
  - evidence: filter input matches the config input on 17 computed properties, contrast 4.99:1 or better in all 10 variants; batch-2 verify-1 gate 4
  - repro: look at the filter box in the Processes panel header at 1440x900 dark
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T11:14:28Z @kj likely cause: app.css gives the input only a border, no background, padding or focus style
  - log: 2026-10-28T11:14:28Z @kj added
  - log: 2026-10-28T15:38:33Z @kj closed
- [x] `DEF-VIEW-50-MEM-TERMS` **Memory terms confusing** - MEDIUM; the Memory panel shows total 503 GiB, used 8.21, available 494, free 481 and cached 12.3 with no visible relation, so the reader cannot tell why available, free and total differ; owner screenshot paste-20261028-120926.png item 3
  - evidence: four-segment memory bar used/cache/other/free sums to 100.000 percent with available = free + cache; batch-2 final gate 2
  - repro: open the Memory panel at 1440x900 and compare the header total with the available, free and cached rows
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T11:14:28Z @kj likely cause: btop's rows are shown without their relation (available = free + reclaimable cache); in container mode some rows may mix host and cgroup values; reproduce first
  - log: 2026-10-28T11:14:28Z @kj added
  - log: 2026-10-28T11:20:53Z @kj owner chose 2026-10-28: stacked bar used + cache + free = total with a legend; available shown once as free + cache; swap below
  - log: 2026-10-28T11:44:20Z @kj owner 2026-10-28: four segments; total = system MemTotal; used = this container's used; other = used by everything else; free = the system's free memory (MemFree); no in-container notion of free
  - log: 2026-10-28T15:38:33Z @kj closed
- [x] `DEF-VIEW-55-CHIP-BLANK` **LIVE chip holds a blank band** - MINOR; the top-bar LIVE chip reserves the width of 'PAUSED since HH:MM:SS', so while live it shows a wide empty band between LIVE and the sample age; empty space is a defect (DEF-LAYOUT-15); screenshot /tmp/claude-1000/-home-lab-workspace-private-btop-web/29fd4ba6-8941-4eba-8147-0d90c052614e/scratchpad/look-dark-1440.png
  - evidence: worst internal blank run 4.00 px over 20 s; chip and eight neighbours drift 0.000 px across LIVE, PAUSED and STALE; both times reachable
  - repro: open the batch-2 build at 1440x900 while live and look at the LIVE chip
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T13:56:47Z @kj acc-74 reserves the widest state text for the whole chip instead of for the changing value
  - log: 2026-10-28T13:56:47Z @kj added
  - log: 2026-10-28T18:03:55Z @kj closed
- [x] `DEF-VIEW-56-NUMBER-INDENT` **CPU big number indented** - MEDIUM; the CPU tile's big number is right-aligned in a slot as wide as 100.0, so 1.0 starts about 45 px right of the panel title and the other tiles' numbers; screenshot /tmp/claude-1000/-home-lab-workspace-private-btop-web/29fd4ba6-8941-4eba-8147-0d90c052614e/scratchpad/look-dark-1440.png
  - evidence: CPU gauge block within 0.055 px of its axis at injected 1, 14 and 100; tile value at the content edge with no reserve indent; drift 0.000 px over 20 s
  - related: ACC-VIEW-81-NUMBER-ALIGN
  - repro: open the batch-2 build at 1440x900 with CPU below 10%; compare the CPU number's left edge with Memory and Processes
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T13:56:47Z @kj acc-74 reserves the widest form left of a short value; the unit position is kept at the cost of a blank band before the number
  - log: 2026-10-28T13:56:47Z @kj added
  - log: 2026-10-28T14:08:22Z @kj owner 15:06 (/tmp/paste-files/paste-20261028-150656.png): the shifted percentage looks bad; same blank band in the CPU gauge centre (14.0%) and before cores busy 9.0 of 64; owner asked for design alternatives
  - log: 2026-10-28T14:08:23Z @kj edited severity "MINOR" -> "MEDIUM"
  - log: 2026-10-28T18:03:55Z @kj closed
- [x] `DEF-VIEW-57-MEM-INDENT` **Memory numbers indented** - MEDIUM; the Memory tile's used number (slot 0000) and the Memory panel's percentages (slot 100%) and MiB values (slot 0000 MiB) are right-aligned in their reserved slots, so short values start after a blank band, as in DEF-VIEW-56-NUMBER-INDENT
  - evidence: memory gauge 0.000 px off axis; unit and used range hold; legend right edges spread 0.000 px; numeric table columns stay right-aligned
  - related: ACC-VIEW-81-NUMBER-ALIGN, DEF-VIEW-56-NUMBER-INDENT - same cause
  - repro: open the page with memory used below 1000 GiB and below 10%; compare each Memory value's left edge with its label
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T14:10:19Z @kj acc-74 [data-w] sets text-align: right (app.css:118), so the reserve sits before the value; mem_stat.html and mem_gauge.html use it
  - log: 2026-10-28T14:10:19Z @kj added
  - log: 2026-10-28T18:03:55Z @kj closed
- [x] `DEF-VIEW-59-ASSET-CACHE` **New build keeps the old stylesheet in the browser** - MEDIUM; app.css, app.js and the theme scripts are served with no Cache-Control, ETag or Last-Modified, so a browser may reuse a stylesheet from an older build after a deploy; serve assets with a validator so a new build shows at once
  - evidence: verify-1 gate 3: /assets carries a quoted sha256 ETag and no-cache, 304 on a matching If-None-Match, and a fresh tab revalidated and applied a changed app.css after a rebuild without a hard reload; commit 9161885
  - repro: deploy a build that changes app.css, keep the browser open, and load the page; the old stylesheet can be reused
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T16:06:14Z @kj added
  - log: 2026-10-28T16:06:58Z @kj edited text "Assets are served with no Cache-Control, ETag or Last-Modified, so a browser reuses the stylesheet it already has while the HTML comes from the new build; after the 0.6.30 deploy the filter input still showed its old yellow placeholder until a reload; serve assets with a validator" -> "app.css, app.js and the theme scripts are served with no Cache-Control, ETag or Last-Modified, so a browser may reuse a stylesheet from an older build after a deploy; serve assets with a validator so a new build shows at once"; repro "deploy a build that changes app.css, then open the page without clearing the cache; the old styles show under the new version" -> "deploy a build that changes app.css, keep the browser open, and load the page; the old stylesheet can be reused"
  - log: 2026-10-28T16:06:58Z @kj measured 2026-10-28: the yellow block in the owner's screenshot is a highlight over the word (pure #fcfd55 with black text, red outline drawn on the image), not the input's style; the running 0.6.30 build renders the placeholder as #909293 in dark Default
  - log: 2026-10-28T19:20:04Z @kj owner saw on 0.6.30: text above the GPU panel, no lane boundary, legend overflow, misplaced memory gauge, no tabs while the squares are gone; proxy and direct CSS are byte-identical (sha 3c5d1c4d), so a stale copy would sit in his browser; the same build opened on port 50561
  - log: 2026-10-28T20:25:45Z @kj closed
- [x] `DEF-VIEW-60-LANE-GAP` **GPU lane border crowds the legend text** - MINOR; The lane boundary rule sits against the first legend character, so border and text touch; the boundary needs clear spacing from the GPU name and from the bars on both sides at every width
  - evidence: GPU lane boundary is a 1 px full-width rule, 1.84:1 dark and 1.74:1 light in all 8 pairs, above the grid and below data lines; verify-1.json gate 1
  - repro: open the GPU panel at 1440x900; the vertical lane border touches the legend text
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T19:24:04Z @kj added
  - log: 2026-10-28T22:02:43Z @kj closed
- [x] `DEF-VIEW-62-FILTER-YELLOW` **Filter placeholder renders yellow** - MINOR; The process filter's placeholder reads yellow instead of the faint grey measured at #909293; the stylesheet has one placeholder rule in --muted and no focus background, so establish what paints it before changing the style
  - evidence: 16/16 pair and mode combos paint the placeholder in --muted; Ctrl+F under Xvfb reproduces the yellow as the browser's find-in-page highlight, no app rule paints it; verify-1.json gate 4
  - repro: open the page in the owner's browser; the filter placeholder shows a yellow block around the word
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T19:24:04Z @kj added
  - log: 2026-10-28T22:02:43Z @kj closed
- [ ] `DEF-VIEW-64-TOOLTIP-DELAY` **Tooltip opens immediately on hover** - MEDIUM; The stat tooltip shows on the first pointermove over a stat; it must wait 3 s of rest on the same stat, and a move off, a click or a re-render must cancel the pending wait
  - repro: hover any stat in the top bar or a panel header; the tooltip box appears at once
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T20:47:53Z @kj added
- [ ] `DEF-VIEW-65-TREE-GLYPHS` **Tree glyphs render malformed** - MINOR; Tree glyphs are mixed and misaligned: one root paints U+250C, another root U+25BE, a depth-1 row U+2502 U+2514, and the vertical bar runs past the last sibling; the column clips at 10 px
  - root-cause: 2026-10-28T20:56:34Z @kj btop 1.3.0 btop_shared.cpp:188 uses fixed 3-char slots: ' │ ' per depth, then ' ├─ ' or ' └─ ' for the last child, '[+]─'/'[-]─' for a node with children, ' ┌─ ' first root; ours glues a bare │ per level and puts the twisty in the branch slot
  - repro: process panel, tree view: compare the glyph before each row with its depth in data-raw
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T20:55:26Z @kj added
  - log: 2026-10-28T20:55:44Z @kj seen again at 391/391 rows: depth-1 children paint a leading vertical bar (U+2502 U+251C) although their parent is the root, and the twisty sits inside the prefix instead of its own column
  - log: 2026-10-28T21:08:56Z @kj owner keeps the triangle as the fold marker: adopt btop's three-character slots and corner branches, with U+25BE/U+25B8 in the branch slot instead of [+]/[-]
  - log: 2026-10-28T21:09:10Z @kj scope is the glyph column only - the characters and their alignment; the fold stays the triangle, no other tree behaviour changes and no new control
  - log: 2026-10-28T21:09:22Z @kj triangle takes a trailing space: U+25B8/U+25BE then space before the name, as in '▾ conda'
  - log: 2026-10-28T21:09:30Z @kj gap after the triangle is tuned in CSS, smaller than a full space (around 0.2em), not a literal space character
  - log: 2026-10-28T21:09:39Z @kj the gap is perceptible, not zero and not a full space - triangle, small gap, name
  - log: 2026-10-28T21:09:52Z @kj use a half-space character after the triangle (U+2002 en space, U+2009 thin space if the font lacks it), measured in the browser so it renders narrower than a normal space
- [ ] `DEF-VIEW-66-TREE-COLLAPSED` **Collapsed tree hides almost every process without a cue** - MEDIUM; With tree nodes collapsed the panel showed 3 of 325 processes with nothing on screen saying 322 rows were hidden; the list reads as broken
  - repro: CONFIG, procs tree, tree nodes collapsed, expand the root: 3 rows
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T20:55:26Z @kj added
- [ ] `DEF-VIEW-67-SPARKLINE-TONE` **Zero process shows a grey sparkline with visible history** - MINOR; A row reading 0.0 keeps a visible sparkline of its decayed history, painted in the zero tone (grey); tone follows the current value while the shape follows the window, so the bar reads as a leftover
  - repro: process list: find a row at 0.0 whose history had a spike; grey bar, no colour
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T21:13:09Z @kj added
- [ ] `DEF-VIEW-68-UNIT-GAP` **Stat number sits far from its unit** - MINOR; The CPU tile shows 3 at the left edge of a reserve as wide as 100, leaving a wide gap before the %; the digits must sit against the unit, with the reserve falling before the number
  - overrides: ACC-VIEW-87-NUMBER-PLACE
  - root-cause: 2026-10-28T21:30:49Z @kj [data-w] in app.css reserves the widest form as a hidden ::before after the number and left-aligns the digits (ACC-VIEW-87-NUMBER-PLACE), so a short value leaves its reserve between the digits and the unit
  - repro: CPU panel, a one-digit total: the % stands about 30 px right of the digits
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T21:30:49Z @kj added
- [ ] `DEF-VIEW-69-GAUGE-GAP` **Gauge unit spacing differs between CPU and Memory** - MINOR; The CPU gauge sets the percent sign against the digits while the Memory gauge leaves a small gap before GiB; both gauges must place the unit at the same distance, from one spacing token
  - related: DEF-VIEW-68-UNIT-GAP
  - repro: CPU and Memory panels side by side at 1440x900: 2% is tight, 21.1 GiB has a gap
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T21:36:43Z @kj added
- [x] `DEF-VIEW-70-ANIM-STATIC` **Animation page shows no motion in the owner's browser** - MAJOR; The ten-concept page animates in headless Chromium (25-34 running animations, 8 of 8 distinct frames) but the owner sees a static page; cause unknown, the served snapshot was also stale until 22:44
  - evidence: root cause found by the owner: his OS turns animations off, so Chrome reports prefers-reduced-motion reduce and the page obeyed it; the rebuilt page shows a motion-state chip and a FORCE MOTION button, which he used and saw the animations
  - repro: Open proxy/41887 in the owner's Chrome, 0.25x, any interval: numbers update, nothing transitions
  - log: 2026-10-28T21:44:52Z @kj added
  - log: 2026-10-28T21:47:15Z @kj closed: closed - environment, page behaves as specified
- [x] `DEF-VIEW-71-PARTICLE-MORPH` **Particle concepts restart from scratch instead of morphing** - MINOR; Block assemble and Particle collapse sample dots from the new digits only, so each change builds a new number from empty space; the dots must start on the previous digits and travel to the new ones so the value morphs
  - evidence: blocks and particles start on the old digits and travel: first-quarter old IoU 0.60-0.75, last-quarter new IoU 0.56-0.84, middle max 0.69; 44 checks pass; served on 41887 sha256 dad8bc85
  - repro: Watch card 02 or 07 at 0.25x through one value change: the old digits vanish, the new ones assemble from scattered dots
  - log: 2026-10-28T21:47:15Z @kj added
  - log: 2026-10-28T22:17:43Z @kj closed
- [ ] `DEF-VIEW-72-MOTION-GATE` **Plots stay static because the OS turns animations off** - MAJOR; app.js motionAllowed() also requires prefers-reduced-motion to be unset, so the owner's machine, which sets it for every page, freezes the charts even with CONFIG motion on; the app's own motion toggle must be the only switch
  - repro: CONFIG motion is on, CONFIG mode dark; a plot does not scroll between samples while the OS animation setting is off
  - log: 2026-10-28T21:50:51Z @kj added
- [ ] `DEF-VIEW-73-LOAD-CRYPTIC` **CPU footer load is three unlabelled integers** - MEDIUM; The CPU footer keeps its three numbers; the hover tooltip, which appears after 3 seconds, must explain them in plain words: tasks runnable or blocked on I/O, averaged over 1, 5 and 15 minutes, not a count of cores; today's tip says only load average, unit tasks
  - repro: look at the CPU panel footer at 1440x900: load 40 37 32
  - log: 2026-10-28T21:56:01Z @kj added
  - log: 2026-10-28T21:56:41Z @kj amended text "templates/partials/cpu_stat.html prints load 40 37 32: no word names the metric, no time span names each value, and {:.0} drops the two decimals the collector keeps, so no reader can tell what the numbers are" -> "The CPU footer keeps its three numbers; the hover tooltip, which appears after 3 seconds, must explain them in plain words: tasks runnable or blocked on I/O, averaged over 1, 5 and 15 minutes, not a count of cores; today's tip says only load average, unit tasks"
- [ ] `DEF-VIEW-74-DOUBLE-TIP` **Container chip shows two tooltips at once** - MEDIUM; templates/index.html gives the CONTAINER chip both a data-tip, drawn by the house tooltip, and a title, drawn by the browser; hovering shows two boxes with different text. An element with data-tip must carry no title
  - repro: hover the CONTAINER chip for a moment: the scope box and the browser's title box appear together
  - root-cause: 2026-10-28T22:07:46Z @kj the chip carries title={{ title }} alongside data-tip in the same element
  - log: 2026-10-28T22:07:46Z @kj added
- [ ] `DEF-VIEW-75-TIP-DWELL` **Tooltip needs cursor movement and opens away from the pointer** - MAJOR; Measured on the live build: the box opens 3.1 s after the pointer arrives with no further movement, but it flips away from the pointer - 68 px above a stat value and 219 px left of the Memory total fact - so it reads as not appearing at the cursor; the placement must follow the pointer and flip only at the viewport edge
  - repro: hover the Memory used value, keep moving the mouse slightly, then stop: the box appears late and away from the cursor
  - log: 2026-10-28T23:42:36Z @kj added
  - log: 2026-10-28T23:43:57Z @kj amended text "On the live page the stat tooltip appears only after several seconds of moving the cursor, and away from it; it must open 3 s after the pointer rests on a stat, at the pointer, and small movement inside the same stat must not restart the wait" -> "Measured on the live build: the box opens 3.1 s after the pointer arrives with no further movement, but it flips away from the pointer - 68 px above a stat value and 219 px left of the Memory total fact - so it reads as not appearing at the cursor; the placement must follow the pointer and flip only at the viewport edge"; reason: measured reproduction differs from the first report; the placement is the confirmed cause

## Process collection `PROC`

Process table collection from btop's Proc::collect

- log: 2026-10-25T03:14:08Z @kj category created

- [x] `DEF-PROC-9-UID` **NSS-only uid shows a number** - MINOR; a uid absent from /etc/passwd renders as the numeric uid; btop tries getpwuid(3) first, and the nix user feature is not enabled
  - evidence: nix user feature enabled; user_name tries /etc/passwd, getpwuid_r, then the numeric uid; a_uid_missing_from_passwd_uses_getpwuid_then_the_number passes
  - repro: run a process under an NSS-only uid and compare the user column with btop
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T03:49:02Z @kj closed

## Disk collection `DISK`

Disk selection and usage from btop's Mem::collect

- log: 2026-10-25T03:14:08Z @kj category created

- [x] `DEF-DISK-11-ESCAPE` **Escaped mountpoint is listed where btop omits it** - MINOR; btop-web decodes octal escapes in /proc/self/mounts and lists the mount; btop passes the raw token to statvfs and ignores it
  - evidence: raw mount token passed to statvfs and unescape_mount deleted; escaped_mountpoint_is_passed_to_statvfs_raw passes; verify temp tree lists only the resolvable mount
  - repro: record a mountpoint containing an escaped space; btop-web lists it, btop does not
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T03:49:02Z @kj closed

## Test fixtures `TEST`

Recorded fixtures that stand in for the host

- log: 2026-10-25T03:14:08Z @kj category created

- [x] `DEF-TEST-13-LOADAVG` **Unavailable fixture shows a zero load average** - MINOR; the fixture's loadavg file contains zeros, so Load 1/5/15 reads 0.00 0.00 0.00 and can be mistaken for an invented zero
  - evidence: zeroed loadavg fixture removed; unavailable_sensors_and_battery_stay_none asserts load_avg None; verify renders Load 1/5/15 unavailable
  - repro: run with BTOP_WEB_PROC_ROOT=crates/btop-web/tests/fixtures/cpu/unavailable/proc and open the CPU panel
  - log: 2026-10-25T03:14:08Z @kj added
  - log: 2026-10-25T03:49:02Z @kj closed
- [x] `DEF-TEST-52-HOST-MOUNT` **Mount test depends on the lab host** - MAJOR; GitHub CI run 40689486618 fails collector::mem::tests::only_physical_filters_by_proc_filesystems (mem.rs:992): the fixture lists /gpu-bookings, the collector calls statvfs on that real path, which exists only on the lab host, so the runner drops the mount
  - evidence: mem tests 20/20 with every fixture mountpoint renamed to a path absent here; make ci 255 tests; tmp/workflows/btop-web-batch3/checkpoints/verify-1.json
  - related: ACC-BUILD-3-CI
  - repro: run the library tests on a machine without /gpu-bookings; the assertion at mem.rs:992 fails
  - test-tags: UNIT
  - root-cause: 2026-10-28T12:41:48Z @kj mem.rs:306 calls statvfs on the live mountpoint during fixture tests; the test asserts a mount whose live path exists only here; breaks the fixtures-not-live-host rule
  - log: 2026-10-28T12:41:48Z @kj added
  - log: 2026-10-28T12:52:52Z @kj fix d-52 ready (wf_7f7efcc2-1ad): statvfs resolves mountpoints under the recorded tree root, live /proc maps to /; mem tests pass 20 of 20 with every asserted mountpoint absent on the host, HEAD fails 3; make ci 246
  - log: 2026-10-28T18:03:54Z @kj closed
- [x] `DEF-TEST-53-HOST-IFACE` **Network tests need a host eth0** - MEDIUM; net::collect lists interfaces through the host's getifaddrs even for a recorded tree, so tests/e2e.rs:282 and tests/http.rs:462 and :1072 pass only on a host with eth0; a developer machine with enp* names fails make ci
  - evidence: net tests 16/16 and the page test pass from the recorded tree with enp0s3 and veth7 on this eth0 host; new fixture added
  - related: DEF-TEST-52-HOST-MOUNT
  - repro: run make ci on a Linux host whose interfaces are named enp0s3 and lo only
  - test-tags: UNIT, FUNCTIONAL
  - root-cause: 2026-10-28T12:52:52Z @kj net.rs reads interface names from getifaddrs, not from the fixture's sys/class/net, when BTOP_WEB_PROC_ROOT points at a recorded tree
  - log: 2026-10-28T12:52:52Z @kj added
  - log: 2026-10-28T18:03:54Z @kj closed

## Libraries `LIB`

Use of maintained libraries and frameworks instead of hand-written standard routines

- log: 2026-10-25T03:51:26Z @kj category created

- [x] `DEF-LIB-14-HAND-ROLLED` **Standard routines are hand-written instead of library calls** - CRITICAL; WCAG luminance and contrast are written twice (src/theme.rs:507 and tests/theme_contrast.rs:13); byte humanisation is bespoke (collector/mod.rs:516); the btop.conf and theme parsers are hand-written; the planned PCHIP smoothing would be another. Standard work uses maintained libraries, with only btop's own dialect kept local
  - evidence: WCAG luminance and contrast call palette 0.7.7 Wcag21RelativeContrast in src/theme.rs and tests/theme_contrast.rs; no hand-written formula remains. humansize, byte-unit, rust-ini, configparser and ini were evaluated and rejected with reasons beside each kept routine; make ci 88 tests green; HSL filed separately
  - repro: grep src and tests for relative_luminance, contrast_ratio and fn human; each is a hand-written copy of a routine a maintained crate provides
  - test-tags: UNIT, FUNCTIONAL
  - root-cause: 2026-10-25T03:51:26Z @kj scaffold built with no dependency policy, so standard routines were typed in place
  - log: 2026-10-25T03:51:26Z @kj added
  - log: 2026-10-25T07:01:09Z @kj closed
- [x] `DEF-LIB-19-HSL-CONVERT` **HSL conversion is hand-written** - MEDIUM; rgb_to_hsl and hsl_to_rgb in src/theme.rs are hand-written colour maths; palette 0.7.7 is already a dependency and provides Hsl conversion. Replace both with palette, keep the derived light theme byte-identical, and keep the contrast tests green
  - evidence: invert_lightness converts through palette 0.7.7 Hsl; rgb_to_hsl and hsl_to_rgb deleted; new pin test asserts the derived Default light block byte-identical (48 raw keys, 15 semantic tokens) before and after; make ci 78 lib, e2e, 7 http, 3 contrast green
  - repro: grep src/theme.rs for rgb_to_hsl and hsl_to_rgb; both implement the standard HSL formulae locally while palette is already linked
  - test-tags: UNIT, FUNCTIONAL
  - root-cause: 2026-10-25T07:01:09Z @kj same scaffold gap as DEF-LIB-14-HAND-ROLLED, left outside that fix's four items
  - log: 2026-10-25T07:01:09Z @kj added
  - log: 2026-10-25T07:03:42Z @kj closed

## Layout `LAYOUT`

Page fit, density and use of space

- log: 2026-10-25T03:54:50Z @kj category created

- [x] `DEF-LAYOUT-15-ONE-PAGE` **The dashboard needs scrolling and wastes space** - CRITICAL; the whole dashboard fits one viewport at 1440x900 and 390x844 with no page scroll. Secondary fields collapse or shrink instead of reserving empty area; empty space is a defect, not breathing room. The current design scrolls and leaves large unused regions
  - evidence: live server measured in headless Chrome: scrollHeight equals innerHeight at 1440x900, 1280x800, 1024x768 and 390x844; no horizontal scroll; the process table is the only scroller; screenshot tmp/workflows/btop-web-ui/shots/ui-live-host-1440x900.png
  - repro: open docs/design/index.html at 1440x900 and at 390x844, compare document.scrollHeight with innerHeight, and measure the blank regions between panels
  - test-tags: E2E, MANUAL
  - root-cause: 2026-10-25T03:54:50Z @kj layout sized to content instead of the viewport, with every field given its own row
  - log: 2026-10-25T03:54:50Z @kj added
  - log: 2026-10-25T07:54:57Z @kj closed
- [x] `DEF-LAYOUT-35-PAUSED-OVERFLOW` **Top bar overflows at 360 px while paused** - MINOR; at 360 px wide in PAUSED state the top bar overflows by 13 px and CONFIG reads CONFI, because the live chip keeps its sample age at the P1 tier; owner chose 2026-10-27: drop the age at P1 in every state
  - evidence: drop-p2 hides the live chip sample age in LIVE, PAUSED and STALE; top bar scrollWidth equals viewport at 360x640 and 390x844, CONFIG in full; make ci 145 green; tmp/workflows/btop-web-round2-fix/checkpoints/final.json
  - repro: open at 360x640, pause in CONFIG: top bar scrollWidth exceeds 360
  - test-tags: FUNCTIONAL
  - log: 2026-10-27T11:51:31Z @kj added
  - log: 2026-10-27T13:27:20Z @kj closed
- [x] `DEF-LAYOUT-36-GPU-DROP` **GPU temperature and power drop without a named tier** - MINOR; at 1280x720, 1366x650 and 1536x730 the GPU legend hides temperature and power through its container step, but no tier chip appears, against ACC-LAYOUT-21 (a drop is named, never silent); owner chose 2026-10-27: name the drop in the tier chip
  - evidence: tier chip names the GPU temperature and power drop (p1-p4) via syncGpuDrop; 0 unnamed drops in 182 sizes (113 before); 1366x650 shows p1-p4 live; make ci 145 green; tmp/workflows/btop-web-round2-fix/checkpoints/final.json
  - repro: open the page at 1366x650: GPU legend lacks temperature and power, no tier chip in the top bar
  - test-tags: FUNCTIONAL
  - log: 2026-10-27T11:51:35Z @kj added
  - log: 2026-10-27T13:27:20Z @kj closed
- [x] `DEF-LAYOUT-58-VERSION-WIDTH` **Longer version overflows the 360 px top bar** - MINOR; each rebuild bumps the patch version; at 0.6.100 (72 bumps from 0.6.28) the version is 7 characters and the CONFIG chip ends about 2.9 px past a 360 px viewport in PAUSED with READ-ONLY on; 0.6.28 already uses 2.1 px of the bar's 4 px right padding
  - evidence: at 360x640 with version 0.6.100, PAUSED and READ-ONLY, no scroll, version fully visible, CONFIG right edge 356.0 at the bar content edge
  - related: ACC-BUILD-84-AUTO-PATCH, ACC-LAYOUT-62-VERSION
  - repro: set the version to 0.6.100, open the page at 360x740, pause it with READ-ONLY on; measure the CONFIG chip's right edge
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T14:25:42Z @kj the drop-p2 tier keeps the version, interval, live chip, READ-ONLY and CONFIG at fixed gaps; docs/design/notes.md:464 predicted it
  - log: 2026-10-28T14:25:42Z @kj added
  - log: 2026-10-28T18:03:55Z @kj closed

## Graphs and sparklines `GRAPH`

Line graphs, sparklines and history charts: interpolation, marks, scale and obstruction

- log: 2026-10-25T06:08:37Z @kj category created

- [x] `DEF-GRAPH-18-OBSCURED-PLOT` **Readout boxes cover the newest graph samples** - MAJOR; the network readout boxes sit over the right edge of both graphs and hide the newest samples. The right edge is where new data arrives and must stay clear; readouts belong in a line above the plot or beside it, never over the plot area
  - evidence: readouts moved to .net-head header lines; .net-plot now holds only the chart svg; CDP check finds no positioned element over any chart; fit 900/900, 800/800, 844/844; screenshot tmp/workflows/btop-web-onepage/shots/onepage-unobscured-1440x900.png
  - repro: open docs/design/onepage.html; the download and upload boxes cover the last fifth of each graph at the right edge
  - test-tags: E2E, MANUAL
  - root-cause: 2026-10-25T06:08:37Z @kj readout overlay copied from btop's box without accounting for the newest edge
  - log: 2026-10-25T06:08:37Z @kj added
  - log: 2026-10-25T06:11:37Z @kj closed
- [x] `DEF-GRAPH-32-UNSTABLE-HISTORY` **Old plot points keep changing shape** - MAJOR; peaks in the history of a plot change height and shape every update, long after they arrived; seen on the network download plot 2026-10-26; likely cause: chart.rs path() evaluates the PCHIP curve at every pixel column, 960 samples over about 391 columns, and each tick shifts the samples 0.4 column, so the columns hit different spots of each peak
  - evidence: cause confirmed by test (old path moved a spike top between y 55.23 and 60.49); knots now one per bucket anchored to the absolute sample count; 0 of about 2,325 completed vertices changed y over 41 live frames; wf_ec680a42-95f tmp/workflows/btop-web-motion2/checkpoints/verify-1.json, tmp/workflows/btop-web-motion2/checkpoints/final-2.json check 2
  - root-cause: 2026-10-26T22:23:05Z @kj path() sampled the PCHIP curve at fixed pixel columns while each 250 ms tick shifts the 960 samples 0.35-0.49 column, so each column hit a different spot of a peak every frame; the spike test drew its top at y 55.23-60.49 over 40 frames before the fix
  - repro: watch a peak 60 s back on the live network plot for 10 s: its height changes between frames
  - test-tags: UNIT, FUNCTIONAL
  - log: 2026-10-26T21:16:50Z @kj added; reason: the cause names the sample count, column count and per-tick shift, which a fix needs
  - log: 2026-10-26T22:22:57Z @kj fix attempt: one knot per bucket anchored to the absolute sample count; 80 live frames, 0 changed vertices outside network auto-scale; awaiting verify
  - log: 2026-10-26T22:23:05Z @kj root-cause updated "2026-10-26T22:22:57Z @kj chart.rs path() sampled the PCHIP curve at fixed pixel columns while each 250 ms tick shifts the 960 samples 0.49 column, so each column hit a different spot of a peak every frame; the spike test drew its top between y 55.23 and 60.49 over 40 frames before the fix" -> "path() sampled the PCHIP curve at fixed pixel columns while each 250 ms tick shifts the 960 samples 0.35-0.49 column, so each column hit a different spot of a peak every frame; the spike test drew its top at y 55.23-60.49 over 40 frames before the fix"
  - log: 2026-10-26T23:16:50Z @kj closed
- [x] `DEF-GRAPH-39-NET-JUMP` **Network graph jumps** - MEDIUM; the Network panel plot jumps while it updates (owner screenshot /tmp/paste-files/paste-20261027-141856.png: eth0, shared scale 773K, upload line with spikes and two drops to the floor, download near the top); which motion jumps is not yet known; owner reported 2026-10-27
  - evidence: 10 s span: 1 rescale in 4 min replay against 28; largest move 3.14 px per frame against 74.55 px; steady left edge; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - repro: open the page, watch the Network panel for 2 minutes at 250 ms
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-27T13:51:44Z @kj net.rs auto scale keeps btop's sample counts (5 counts net.rs:287, mean of 5 net.rs:294-295) at 250 ms instead of 2000 ms: 28 rescales in 4 min vs 4 at 2000 ms; each rescale moves every vertex of both plots (up to 67.8 of 77.5 px). Secondary: first vertex is the oldest sample, replaced every tick (chart.rs:184), left 2 units move
  - root-cause: 2026-10-27T13:19:46Z @kj cause unknown; reproduce first: candidates are scale changes, knots moving between frames, counter updates slower than the 250 ms tick, frame gaps
  - log: 2026-10-27T13:19:46Z @kj added
  - log: 2026-10-27T13:51:44Z @kj root-cause overridden; reason: two causes with sites and numbers
  - log: 2026-10-27T13:51:44Z @kj investigated wf_43b1b0ae-82c: replay 64/64 rescales exact; fix proposal n = max(5, 10000/update_ms) gives 1 rescale in 4 min; evidence tmp/workflows/btop-web-net-jump/record.md, checkpoints/refute.json
  - log: 2026-10-27T18:20:32Z @kj owner decided 2026-10-27: auto scale counts over btop's 10 s span at any interval, n = max(5, 10000/update_ms); steady left edge: once the window is full the line starts at the first complete bucket, every chart
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint
- [x] `DEF-GRAPH-51-GPU-LANES` **GPU lines share one plot** - MEDIUM; the GPU load graph draws every GPU in one plot, so lines overlap (three GPUs at 0% show as one line); each GPU needs its own lane on the shared time axis; owner screenshot paste-20261028-120926.png item 4
  - evidence: one lane per GPU; batch-2 verify-1 gate 1: 3 lanes, 3 chart boxes, each legend entry centred on its plot within 0.01 px, one shared axis
  - related: ACC-GRAPH-78-GPU-LANES
  - repro: open the GPU panel with three idle GPUs; one line is visible
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T11:14:28Z @kj by design: ACC-GRAPH-43-GPU-LOAD specified one plot with one series per GPU; the owner now wants one lane per GPU
  - log: 2026-10-28T11:14:28Z @kj added
  - log: 2026-10-28T11:44:20Z @kj owner 2026-10-28: replace the one shared GPU graph with three separate graphs, one per GPU, each 0-100%; the legend keeps temperature and power at 1024x768 too
  - log: 2026-10-28T15:38:34Z @kj closed
- [x] `DEF-GRAPH-54-LANES-MERGED` **GPU graphs still read as one** - MAJOR; the three GPU lane boxes sit 2 px apart and their vertical grid lines run on without a break, so the panel still reads as one graph; each GPU graph needs a visible boundary of its own; screenshot /tmp/claude-1000/-home-lab-workspace-private-btop-web/29fd4ba6-8941-4eba-8147-0d90c052614e/scratchpad/look-dark-1440.png
  - evidence: three lanes with equal 4 px gaps and a 1 px rule, vertical grid inside each lane, 0 and 100 percent strokes 2.5-3.5 px clear of the edges, at three sizes dark and light
  - related: ACC-GRAPH-78-GPU-LANES - the criterion this defect breaks
  - repro: open the batch-2 build at 1440x900 and look at the GPU panel
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-28T13:56:47Z @kj d-51 separates the lanes only by a 2 px gap; same grid colour and no frame per lane
  - log: 2026-10-28T13:56:47Z @kj added
  - log: 2026-10-28T14:03:10Z @kj owner confirmed on the live page 15:02 (/tmp/paste-files/paste-20261028-150223.png): one must see where one lane ends and the next begins; GPU 0 at 0% draws on its lane's bottom edge, 7 px above GPU 1's 100% line at the next lane's top edge, so the two lines read as one pair
  - log: 2026-10-28T14:03:10Z @kj edited severity "MEDIUM" -> "MAJOR"
  - log: 2026-10-28T18:03:54Z @kj closed
- [x] `DEF-GRAPH-61-NET-LANES` **Network plot does not separate download and upload** - MEDIUM; The network plot draws download and upload in one undivided box; each needs its own lane with a visible boundary, as the GPU panel has, on the shared time axis
  - evidence: download above upload, one boundary equal to the GPU rule in 32/32 cases, shared x axis, header scale equals the gutter totals; verify-1.json gate 2
  - repro: open the Network panel; the two series share one plot with no boundary between them
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T19:24:04Z @kj added
  - log: 2026-10-28T22:02:43Z @kj closed
- [x] `DEF-GRAPH-63-GRID-UNITS` **Grids carry no unit labels or corner total** - MEDIUM; The GPU and network grids show no value labels, so the reader cannot tell the scale; label a few horizontal grid lines with their value and unit and print the total scale in the plot corner
  - evidence: 160 gutters sit outside their chart box, 384 label contrast checks reach 4.52:1, 32/32 probes move no plot; verify-1.json gate 3
  - repro: open the GPU and Network panels; no grid line is labelled and no corner states the scale
  - test-tags: FUNCTIONAL
  - log: 2026-10-28T19:24:04Z @kj added
  - log: 2026-10-28T22:02:43Z @kj closed

## Theme `THEME`

Palette tokens, variants and contrast across the shipped pairs

- log: 2026-10-25T07:57:01Z @kj category created

- [x] `DEF-THEME-20-LIGHT-GREEN` **Green text fails AA on light themes** - MAJOR; process and GPU percentages painted with the ok token fall below 4.5:1 on light variants: 3.04:1 in Everforest and Solarized light, 4.36 to 4.43 in Default, Flexoki and Gruvbox light. Dark variants pass. The token or its light value must reach 4.5:1 for small text
  - evidence: light ok/warn/crit derive at the 4.5:1 text floor in src/theme.rs; tests/theme_contrast.rs light_status_tokens_reach_4_5_to_1 passes for all five light variants; headless Chrome live .cpu-ok text measures 4.52-4.55:1 on the five light variants, dark unchanged; make ci 104 tests green
  - repro: open the page on a light variant and measure the green 0.9% process text against the surface: 3.04 to 4.43 to 1
  - test-tags: UNIT, E2E, MANUAL
  - root-cause: 2026-10-25T07:57:01Z @kj the semantic ok token was derived for dark surfaces and reused unchanged as text on light surfaces
  - log: 2026-10-25T07:57:01Z @kj added
  - log: 2026-10-25T08:10:19Z @kj closed
- [x] `DEF-THEME-21-TINT-CONTRAST` **Text falls below 4.5:1 on tinted layers** - MAJOR; live run over 10 theme and mode pairs: zebra rows 4.209:1 default dark, net arrows 2.049:1 solarized light, chip text 4.459:1 everforest light; tokens meet 4.5:1 only on the flat surface
  - evidence: ten live theme and mode combinations at 1440x900: 0 below-floor runs, lowest 4.512:1 solarized light; composed-layer test text_on_tinted_layers_reaches_4_5_to_1 passes; make ci 105 tests
  - repro: headless Chrome at 1440x900 on the live page, compose each run's background through its ancestors, compare with 4.5:1
  - root-cause: 2026-10-25T08:41:30Z @kj chart colours and text tokens are contrasted against the flat surface, but rows carry a 3% tint, chips 12%, and the net arrows use the raw graph colours as glyph colour
  - log: 2026-10-25T08:41:30Z @kj added
  - log: 2026-10-25T13:49:42Z @kj closed
- [x] `DEF-THEME-22-POPOVER-CONTRAST` **Config popover text falls below 4.5:1** - MAJOR; with the menu open at 1440x900: .cfg-label muted on --surface-raised reads 4.195 solarized dark to 4.398 default dark, and .opt.on text-strong on a 12% accent tint over surface-raised reads 4.427 everforest light; the closed page passes
  - evidence: popover open and closed at 1440x900 across ten theme and mode combinations: 0 below-floor runs; popover worst 4.539:1 everforest light button.opt.on, page worst 4.512:1; composed test extended
  - repro: open the CONFIG popover at 1440x900 in headless Chrome, compose each label's background through the 3% surface-raised layer and the 12% chip tint, compare with 4.5:1
  - root-cause: 2026-10-25T13:49:42Z @kj muted text sits on --surface-raised, which notes.md itself forbids; app.css blends surface-raised at 3% while notes.md records 2%
  - log: 2026-10-25T13:49:42Z @kj added
  - log: 2026-10-25T14:01:48Z @kj closed
- [x] `DEF-THEME-38-SELECT-POPUP` **Theme dropdown list unreadable** - MEDIUM; the open theme list in the config popover shows light grey names on a white list in the dark Default theme, so the names cannot be read; screenshot /tmp/paste-files/paste-20261027-130641.png; owner reported 2026-10-27
  - evidence: select color-scheme follows data-mode, options from theme tokens at 4.5:1, 20 of 20 scheme and mode cases; merge-1 wf_c4d605ea-713 final 8 of 8, make ci 214 green, live 40290 94db7ec0; tmp/workflows/btop-web-merge-1/checkpoints/final-1.json
  - repro: dark mode, Windows Chrome with a light OS theme: open CONFIG, open the theme dropdown
  - test-tags: FUNCTIONAL
  - root-cause: 2026-10-27T12:11:28Z @kj likely cause: no color-scheme follows data-mode, so the meta color-scheme 'dark light' lets the OS light scheme draw the native list white while option text inherits the dark theme's light text colour (app.css:118)
  - log: 2026-10-27T12:11:28Z @kj added
  - log: 2026-10-27T19:41:48Z @kj owner 20:4x reported it again on build 44a72c34; fix d-38 is in the proc-tile run wf_8ca31f9f-9b8 (building), not yet deployed
  - log: 2026-10-28T02:00:20Z @kj closed; reason: evidence names the measured result and the checkpoint

## Documentation `DOCS`

- log: 2026-10-25T14:01:51Z @kj category created

- [x] `DEF-DOCS-24-STALE-AGENTS` **AGENTS.md state block contradicts the tree** - MEDIUM; the State, 2026-10-25 block says the collectors are stubs returning None and quotes lib 14 / e2e 1 / http 5 / theme 3; the tree has 105 tests and live collectors, so the first file every agent reads is wrong
  - evidence: verify gate: every claim in the refreshed State block matches the tree - 110 tests = 94+1+10+5, FEATURES empty by default, rocm off, port 40290, themes 0..4, contrast contract
  - repro: read .agents/AGENTS.md Project Context against make ci output
  - root-cause: 2026-10-25T14:01:51Z @kj the block was written at scaffold time and never refreshed as the collectors and tests landed
  - log: 2026-10-25T14:01:51Z @kj added
  - log: 2026-10-25T15:15:43Z @kj closed
- [x] `DEF-DOCS-29-SHOT-PRIVACY` **README screenshots show private process rows** - MAJOR; docs/design/shipped-dark.png and shipped-light.png show the live process table with Claude session ids, --dangerously-skip-permissions and the path of another private project under /home/lab/workspace/private; the README would publish them
  - evidence: shipped-dark.png and shipped-light.png re-rendered with the page filter on mlflow.server; visible rows asserted free of /home paths, project names, UUIDs and user-tool flags; tmp/workflows/btop-web-gpu-load/checkpoints/docs.json, tmp/workflows/btop-web-gpu-load/docs/shots.json, tmp/workflows/btop-web-gpu-load/checkpoints/final-1.json check 5
  - repro: open docs/design/shipped-dark.png: Command column rows
  - test-tags: MANUAL
  - log: 2026-10-26T17:08:12Z @kj added
  - log: 2026-10-26T18:59:43Z @kj closed
- [x] `DEF-DOCS-30-NOTES-STALE` **notes.md sections describe the earlier design** - MEDIUM; docs/design/notes.md header Graphs row, 7.1 and 7.3 (120 knots, no EMA), 8.1 (status strip, 500-2000 ms), 11.2 and 11.3 still describe the earlier page; the 9.3 table omits that max-height 860px and max-width 1100px also apply at 390x844
  - evidence: notes.md header rows, 7.1, 7.3, 8.1, 9.1-9.5, 11.2, 11.3 and 15 match the shipped page; 9.3 names 390x844 for both breakpoints; tmp/workflows/btop-web-gpu-load/checkpoints/docs.json, tmp/workflows/btop-web-gpu-load/checkpoints/final-1.json check 5
  - repro: read docs/design/notes.md sections 7.1, 8.1, 9.3, 11.2
  - test-tags: MANUAL
  - log: 2026-10-26T17:08:12Z @kj added
  - log: 2026-10-26T18:59:43Z @kj closed
- [x] `DEF-DOCS-31-NOTES-PROOF` **notes.md still describes the index.html proof** - MINOR; docs/design/notes.md opening line, section 8 past 8.1 (999 px pill badge 8.3, 0 0 1000 1000 viewBox 8.7, GPU checkboxes 8.10), sections 10 and 14 describe the earlier index.html proof; .agents/AGENTS.md Open still lists the done value diff and ten-client benchmark
  - evidence: proof-era sections marked Historical and AGENTS.md Open line current (tmp/workflows/btop-web-motion2/checkpoints/final-2.json check 5 parts i-iii); the four remaining false statements plus five more in notes.md corrected and verified against the templates (tmp/workflows/btop-web-docs-fix/checkpoints/verify.json, wf_836da79c-991)
  - repro: read docs/design/notes.md 8.3, 8.7, 8.10, 10, 14 and the AGENTS.md Open line
  - test-tags: MANUAL
  - log: 2026-10-26T18:59:48Z @kj added
  - log: 2026-10-26T22:48:49Z @kj fix attempt: notes.md opening, 6 (motion, sweep, NO_GO, EMA arc), 8.2-8.13, 10 and 14 brought to the shipped page, 4, 5 and appendices B, C marked historical; AGENTS.md Open line cleared; awaiting verify
  - log: 2026-10-26T23:04:46Z @kj fix attempt 2: README Fig 1 and 2 re-rendered from the final-1 build (TOTAL, core rows, load per core); notes.md 1.1 battery, 3.2 role cells, 3.3 token sentence, 12.1 Cpu% and MemB, 12.2 CPU breakdown and Uptime rows corrected; awaiting verify
  - log: 2026-10-26T23:19:51Z @kj closed
- [x] `DEF-DOCS-33-TEST-COUNT` **README states 141 tests, build runs 146** - MINOR; README.md lines 5 and 167 say make ci runs 141 tests (114 library, 19 HTTP); after the process graphs it runs 146 (118 library, 2 end-to-end, 20 HTTP, 6 theme contrast); cause: the graph workflow's docs step did not recount
  - evidence: README test counts set to the make ci result, 148 tests (118 library, 2 e2e, 22 HTTP, 6 theme contrast); final-1 check 5 passes; tmp/workflows/btop-web-adaptive-ship/checkpoints/final-1.json
  - repro: make ci; compare the count with README.md lines 5 and 167
  - test-tags: MANUAL
  - log: 2026-10-27T05:55:54Z @kj added
  - log: 2026-10-27T07:29:40Z @kj closed

## Performance `PERF`

CPU, memory and wire cost of the server and the page against their recorded baselines

- log: 2026-10-29T00:22:32Z @kj category created

- [ ] `DEF-PERF-76-SERVER-COST` **Server costs 2.2x its recorded CPU per tick** - MEDIUM; one SSE client at 250 ms now costs 19.5 ms of server CPU per tick, 7.8 percent of one core, against the 6.66 and 8.8 ms recorded on 2026-10-26; the GPU sweep adds about 40 ms once a second and the process set is 374 rows
  - repro: attach one SSE client to the deployed build at 250 ms for 60 s and read utime+stime from /proc/<pid>/stat; tmp/workflows/btop-web-vs-btop/checkpoints/measure.json
  - root-cause: 2026-10-29T00:22:32Z @kj unknown; likely the NVML sweep at most once a second plus the 374-row process walk, both inside the tick
  - log: 2026-10-29T00:22:32Z @kj added
  - log: 2026-10-29T00:40:31Z @kj second measurement 2026-10-29: 18.2 ms per tick idle and 19.2 ms with one client, against 8.3 and 8.8 ms recorded on 2026-10-26, so growth is 2.1 to 2.2 times; the 1.92 ms figure is collection only, not the whole tick
  - log: 2026-10-29T01:12:33Z @kj cause: process walk 9.3 ms of 18.2 per tick over 390 rows, all GPU work 5.5 (holder fd walk 3.0-3.4), fixed collection 3.3, load count 1.4, one client 0.8, charts under 0.3; the sweep claim was 2-5x too high
- [ ] `DEF-PERF-77-PAGE-COST` **One open tab uses half a core in headless Chromium** - MEDIUM; one headless tab of the dashboard spends 33.1 s of renderer CPU per 60 s, 0.55 of one core, with 14.7 s of main-thread tasks and 59.7 fps; headless Chromium rasterises on the CPU, so a GPU-accelerated browser may cost less; unmeasured
  - repro: open the dashboard in headless Chromium at 1440x900 for 60 s and read the renderer process CPU from /proc; tmp/workflows/btop-web-perf/checkpoints/measure.json
  - root-cause: 2026-10-29T00:40:43Z @kj unknown; likely software rasterisation of four chart swaps per second plus the number-effect frame loop; a GPU-accelerated run and a per-feature profile are needed
  - log: 2026-10-29T00:40:43Z @kj added
  - log: 2026-10-29T01:12:33Z @kj cause: at 1440x900 the tab uses 26.34 s renderer CPU per 60 s; chart scroll from a 60 fps style loop 13.26 s, event stream 11.45 s with the process panel 6.35, stale page 1.63; number effects off, spring blur adds 28.33

