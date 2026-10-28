//! Container scope (ACC-SCOPE-64-CONTAINER): detection, and the cgroup v2
//! readings the collectors share inside a container.
//!
//! btop has no container mode. Inside a container btop-web departs from btop
//! 1.3.0 parity on the owner's order (2026-10-27): every panel shows only this
//! container, and a metric with no container-scoped source renders as
//! unavailable, never as the host value and never as 0. Outside a container
//! nothing here runs and every collector keeps its btop path.
//!
//! Detection runs once at startup on the tree `BTOP_WEB_PROC_ROOT` selects.
//! `<root>` is the parent of `<proc>` (`/` live), `<sys>` is [`sys_root`]. No
//! signal reads the process environment through `std::env`, so tests stay
//! hermetic. The first signal that fires wins:
//!
//! 1. `<root>/.dockerenv` exists: `docker` (Docker creates it in every
//!    container, the containerd image store included)
//! 2. `<root>/run/.containerenv` exists: `podman`
//! 3. `container=<v>` in `<proc>/self/environ`: `<v>`
//! 4. `container=<v>` in `<proc>/1/environ`, when readable: `<v>` (LXC, Incus)
//! 5. `KUBERNETES_SERVICE_HOST=` in `<proc>/self/environ`: `kubernetes`
//! 6. a runtime segment in the `0::<path>` line of `<proc>/self/cgroup`:
//!    `docker`, `kubepods*`, `lxc.payload*`, or `<p>-<hex id>.scope` with `<p>`
//!    in docker, libpod, crio, cri-containerd, nerdctl; named from the segment
//! 7. the `/` mount in `<proc>/self/mountinfo` is `overlay` with `containerd`,
//!    `/docker/` or `/containers/storage/` in its options, or
//!    `fuse.fuse-overlayfs`: `container`. `mountinfo`, never `mounts`: the host
//!    fixture `tests/fixtures/proc/self/mounts` was recorded in a container
//!
//! The container cgroup is `<sys>/fs/cgroup/<path>` from the `0::<path>` line,
//! whichever signal fired (`0::/` under a private cgroup namespace is
//! `<sys>/fs/cgroup` itself). With no `0::` line (cgroup v1) or no such
//! directory it is `None`, container mode stays on and every cgroup-sourced
//! metric is unavailable. The network namespace is the host's when an
//! interface in `<sys>/class/net` other than `lo` has a `device` link (a
//! bus-backed NIC); veth, macvlan, ipvlan, tap and tun have none.
//!
//! Effective CPUs E, read every sample by the CPU and process collectors:
//! E = min(N, quota / period), N = the CPUs listed in `cpuset.cpus.effective`
//! (the online count when the file is missing), quota and period from
//! `cpu.max` (`max` or a missing file: no quota). The container's cores are
//! the CPUs `cpuset.cpus.effective` lists ([`cpuset`], DEF-CPU-41).
//!
//! Own parsing: no crate in `Cargo.lock` reads these files, each is one line
//! or flat key-value text, and every path must follow `BTOP_WEB_PROC_ROOT`.

use std::path::{Path, PathBuf};

use super::sys_root;

/// The detection result, fixed for the process lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// The runtime the signal names: `docker`, `podman`, `kubernetes`, ...
    pub runtime: String,
    /// The signal that fired, its number and name: `1 /.dockerenv`.
    pub signal: String,
    /// The container's cgroup v2 directory; `None` on cgroup v1.
    pub cgroup: Option<PathBuf>,
    /// The network namespace is the container's own, not the host's.
    pub own_netns: bool,
}

impl Container {
    /// The CONTAINER chip's title: runtime, signal, cgroup and what is
    /// unavailable here.
    pub fn title(&self) -> String {
        let cgroup = self.cgroup.as_ref().map_or_else(
            || "unavailable".to_string(),
            |path| path.display().to_string(),
        );
        let mut unavailable = String::new();
        if self.cgroup.is_none() {
            unavailable.push_str("CPU, memory, swap, processes, per-core usage, load, ");
        }
        unavailable.push_str("CPU frequency and temperature, disk IO activity");
        if !self.own_netns {
            unavailable.push_str(", network");
        }
        format!(
            "{} container, detected by {}, cgroup {cgroup}. Unavailable here: {unavailable}",
            self.runtime, self.signal
        )
    }
}

/// Detection over the tree `proc_root` selects; `None` on a host.
pub fn detect(proc_root: &Path) -> Option<Container> {
    let root = match proc_root.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("/"),
    };
    let environ = read_lossy(&proc_root.join("self/environ"));
    let cgroup_line = read_lossy(&proc_root.join("self/cgroup"));
    let cgroup_path = cgroup_line
        .lines()
        .find_map(|line| line.strip_prefix("0::"));
    let (runtime, signal) = if root.join(".dockerenv").exists() {
        ("docker".to_string(), "1 /.dockerenv")
    } else if root.join("run/.containerenv").exists() {
        ("podman".to_string(), "2 /run/.containerenv")
    } else if let Some(value) = env_value(&environ, "container") {
        (value, "3 container= in own environment")
    } else if let Some(value) = env_value(&read_lossy(&proc_root.join("1/environ")), "container") {
        (value, "4 container= in pid 1 environment")
    } else if env_value(&environ, "KUBERNETES_SERVICE_HOST").is_some() {
        ("kubernetes".to_string(), "5 KUBERNETES_SERVICE_HOST")
    } else if let Some(runtime) = cgroup_path.and_then(cgroup_runtime) {
        (runtime.to_string(), "6 cgroup path")
    } else if overlay_root(&read_lossy(&proc_root.join("self/mountinfo"))) {
        ("container".to_string(), "7 container-store root mount")
    } else {
        return None;
    };
    let sys = sys_root(proc_root);
    let cgroup = cgroup_path.and_then(|path| {
        let relative = path.trim().trim_start_matches('/');
        let mut directory = sys.join("fs/cgroup");
        if !relative.is_empty() {
            directory.push(relative);
        }
        directory.is_dir().then_some(directory)
    });
    Some(Container {
        runtime,
        signal: signal.to_string(),
        cgroup,
        own_netns: own_network_namespace(&sys),
    })
}

/// The CPUs `cpuset.cpus.effective` lists, read now; `None` when the file is
/// missing or empty.
pub fn cpuset(cgroup: &Path) -> Option<Vec<usize>> {
    cpu_list(&std::fs::read_to_string(cgroup.join("cpuset.cpus.effective")).ok()?)
}

/// E for the container cgroup, read now.
pub fn effective_cpus(cgroup: &Path) -> f64 {
    effective(
        std::fs::read_to_string(cgroup.join("cpuset.cpus.effective"))
            .ok()
            .as_deref(),
        std::fs::read_to_string(cgroup.join("cpu.max"))
            .ok()
            .as_deref(),
        super::proc::core_count(),
    )
}

/// E = min(N, quota / period); N falls back to `online`.
fn effective(cpuset: Option<&str>, cpu_max: Option<&str>, online: usize) -> f64 {
    let listed = cpuset.and_then(cpu_list).map(|cpus| cpus.len());
    let cpus = listed.unwrap_or(online) as f64;
    let quota = cpu_max.and_then(|text| {
        let mut fields = text.split_whitespace();
        let quota: f64 = fields.next()?.parse().ok()?;
        let period: f64 = fields.next()?.parse().ok()?;
        (period > 0.0).then_some(quota / period)
    });
    quota.map_or(cpus, |quota| quota.min(cpus))
}

/// The CPUs of a kernel list such as `0-3,8,10-11`; `None` when empty or
/// malformed.
fn cpu_list(text: &str) -> Option<Vec<usize>> {
    let mut cpus = Vec::new();
    for range in text.trim().split(',').filter(|range| !range.is_empty()) {
        let (first, last) = range.split_once('-').unwrap_or((range, range));
        cpus.extend(first.parse::<usize>().ok()?..=last.parse().ok()?);
    }
    (!cpus.is_empty()).then_some(cpus)
}

/// A CPU count with at most two decimals and no trailing zeros: `64`, `1.5`.
pub fn cpus_label(cpus: f64) -> String {
    let text = format!("{cpus:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn read_lossy(path: &Path) -> String {
    std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// The non-empty value of `key` in a NUL-separated environment block.
fn env_value(environ: &str, key: &str) -> Option<String> {
    environ
        .split('\0')
        .find_map(|entry| entry.strip_prefix(key)?.strip_prefix('='))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Signal 6: the runtime a cgroup v2 path segment names.
fn cgroup_runtime(path: &str) -> Option<&'static str> {
    path.split('/').find_map(|segment| {
        if segment == "docker" {
            return Some("docker");
        }
        if segment.starts_with("kubepods") {
            return Some("kubernetes");
        }
        if segment.starts_with("lxc.payload") {
            return Some("lxc");
        }
        let (prefix, id) = segment.strip_suffix(".scope")?.rsplit_once('-')?;
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        match prefix {
            "docker" => Some("docker"),
            "libpod" => Some("podman"),
            "crio" => Some("cri-o"),
            "cri-containerd" => Some("containerd"),
            "nerdctl" => Some("nerdctl"),
            _ => None,
        }
    })
}

/// Signal 7: a container-store filesystem mounted at `/`.
fn overlay_root(mountinfo: &str) -> bool {
    mountinfo.lines().any(|line| {
        if line.split_whitespace().nth(4) != Some("/") {
            return false;
        }
        let Some((_, tail)) = line.split_once(" - ") else {
            return false;
        };
        let mut fields = tail.split_whitespace();
        let fstype = fields.next().unwrap_or_default();
        let options = fields.nth(1).unwrap_or_default();
        fstype == "fuse.fuse-overlayfs"
            || (fstype == "overlay"
                && ["containerd", "/docker/", "/containers/storage/"]
                    .iter()
                    .any(|marker| options.contains(marker)))
    })
}

/// No interface other than `lo` has a `device` link: a dangling link counts.
fn own_network_namespace(sys: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(sys.join("class/net")) else {
        return true;
    };
    !entries.flatten().any(|entry| {
        entry.file_name() != "lo" && entry.path().join("device").symlink_metadata().is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(relative: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(relative)
    }

    /// An empty `<root>/proc` with the given files, for one signal at a time.
    fn tree(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("btop-web-container-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proc/self")).expect("temp tree");
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            std::fs::write(path, text).expect("write");
        }
        root
    }

    fn detect_in(root: &Path) -> Option<Container> {
        let found = detect(&root.join("proc"));
        let _ = std::fs::remove_dir_all(root);
        found
    }

    #[test]
    fn docker_marker_detects_docker_with_the_root_cgroup() {
        let found = detect(&fixture("container/proc")).expect("container");
        assert_eq!(found.runtime, "docker");
        assert_eq!(found.signal, "1 /.dockerenv");
        assert_eq!(found.cgroup, Some(fixture("container/sys/fs/cgroup")));
        assert!(found.own_netns);
        assert!(found.title().contains("cgroup "), "{}", found.title());
    }

    #[test]
    fn podman_marker_detects_podman() {
        let root = tree("podman", &[("run/.containerenv", b"")]);
        let found = detect_in(&root).expect("container");
        assert_eq!(
            (found.runtime.as_str(), found.signal.as_str()),
            ("podman", "2 /run/.containerenv")
        );
    }

    #[test]
    fn container_env_detects_its_value() {
        let root = tree(
            "env",
            &[("proc/self/environ", b"PATH=/bin\0container=oci\0")],
        );
        let found = detect_in(&root).expect("container");
        assert_eq!(found.runtime, "oci");
        assert!(found.signal.starts_with("3 "));
    }

    #[test]
    fn pid1_container_env_detects_lxc() {
        let root = tree("pid1", &[("proc/1/environ", b"container=lxc\0")]);
        let found = detect_in(&root).expect("container");
        assert_eq!(found.runtime, "lxc");
        assert!(found.signal.starts_with("4 "));
    }

    #[test]
    fn kubernetes_env_detects_kubernetes() {
        let root = tree(
            "k8s",
            &[("proc/self/environ", b"KUBERNETES_SERVICE_HOST=10.0.0.1\0")],
        );
        let found = detect_in(&root).expect("container");
        assert_eq!(found.runtime, "kubernetes");
        assert!(found.signal.starts_with("5 "));
    }

    #[test]
    fn cgroup_runtime_segment_detects_and_names_the_scope_directory() {
        let scope = "system.slice/docker-0123abcd.scope";
        let root = tree(
            "cgroup",
            &[
                ("proc/self/cgroup", format!("0::/{scope}\n").as_bytes()),
                (&format!("sys/fs/cgroup/{scope}/cgroup.procs"), b"1\n"),
            ],
        );
        let cgroup = root.join("sys/fs/cgroup").join(scope);
        let found = detect_in(&root).expect("container");
        assert_eq!(found.runtime, "docker");
        assert!(found.signal.starts_with("6 "));
        assert_eq!(found.cgroup, Some(cgroup));
        assert_eq!(cgroup_runtime("/kubepods.slice/pod1"), Some("kubernetes"));
        assert_eq!(
            cgroup_runtime("/machine.slice/libpod-ab12.scope"),
            Some("podman")
        );
        assert_eq!(cgroup_runtime("/user.slice/session-1.scope"), None);
    }

    #[test]
    fn containerd_overlay_root_detects_a_container() {
        let mountinfo = std::fs::read(fixture("container/proc/self/mountinfo")).expect("fixture");
        let root = tree("overlay", &[("proc/self/mountinfo", &mountinfo)]);
        let found = detect_in(&root).expect("container");
        assert_eq!(found.runtime, "container");
        assert!(found.signal.starts_with("7 "));
        // No 0:: line: container mode with every cgroup metric unavailable.
        assert_eq!(found.cgroup, None);
        assert!(found.title().contains("CPU, memory"), "{}", found.title());
    }

    #[test]
    fn host_fixture_tree_is_host() {
        assert_eq!(detect(&fixture("proc")), None);
    }

    #[test]
    fn effective_cpus_takes_the_smaller_of_cpuset_and_quota() {
        assert_eq!(effective(Some("0-3\n"), Some("200000 100000\n"), 64), 2.0);
        assert_eq!(effective(Some("0-63\n"), Some("max 100000\n"), 64), 64.0);
        assert_eq!(effective(Some("0-3,8-11\n"), None, 64), 8.0);
        assert_eq!(effective(Some("0-3\n"), Some("50000 100000\n"), 64), 0.5);
        assert_eq!(effective(None, None, 64), 64.0);
        assert_eq!(effective(Some("\n"), None, 64), 64.0);
        assert_eq!(cpu_list("0-1,4\n"), Some(vec![0, 1, 4]));
        assert_eq!(
            cpuset(&fixture("container/sys/fs/cgroup")),
            Some(vec![0, 1, 2, 3])
        );
        assert_eq!(effective_cpus(&fixture("container/sys/fs/cgroup")), 2.0);
        assert_eq!(cpus_label(64.0), "64");
        assert_eq!(cpus_label(1.5), "1.5");
        assert_eq!(cpus_label(0.25), "0.25");
    }

    #[test]
    fn network_namespace_is_own_without_device_links() {
        assert!(own_network_namespace(&fixture("container/sys")));
    }

    #[test]
    fn network_namespace_is_host_with_a_device_link() {
        let root = tree(
            "netns",
            &[
                ("sys/class/net/lo/ifindex", b"1\n"),
                ("sys/class/net/eth0/device/vendor", b"0x8086\n"),
            ],
        );
        let own = own_network_namespace(&root.join("sys"));
        let _ = std::fs::remove_dir_all(&root);
        assert!(!own);
    }
}
