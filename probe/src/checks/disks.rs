//! The `disks` check (M5-05, D-81): every fixed disk of the probe's machine
//! in one check, found each time it runs. A disk added later is watched
//! without anyone adding a check.
//!
//! One check has one result, so it is the **fullest** disk's: its usage is
//! the headline the hub's disk thresholds judge, and the message lists every
//! disk, fullest first.

use common::checks::{Metric, metric};
use sysinfo::Disks;

use super::{BYTES_PER_GB, Outcome, files};

/// How many disks the message names (the rest are counted).
const DISKS_NAMED: usize = 6;

/// File systems that aren't a disk of this machine to run full: read-only
/// images, memory, and other machines' disks.
const NOT_FIXED: [&str; 16] = [
    "squashfs",
    "iso9660",
    "udf",
    "tmpfs",
    "devtmpfs",
    "ramfs",
    "efivarfs",
    "autofs",
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "9p",
    "fuse.sshfs",
    "fuse.rclone",
];

/// One mounted volume, as the system describes it.
#[derive(Debug, Clone, PartialEq)]
struct Volume {
    mount: String,
    device: String,
    file_system: String,
    total: u64,
    free: u64,
    removable: bool,
    read_only: bool,
}

impl Volume {
    fn usage(&self) -> f64 {
        (self.total - self.free.min(self.total)) as f64 / self.total as f64 * 100.0
    }
}

fn all(disks: &Disks) -> Vec<Volume> {
    disks
        .iter()
        .map(|d| Volume {
            mount: d.mount_point().to_string_lossy().into_owned(),
            device: d.name().to_string_lossy().into_owned(),
            file_system: d.file_system().to_string_lossy().to_lowercase(),
            total: d.total_space(),
            free: d.available_space(),
            removable: d.is_removable(),
            read_only: d.is_read_only(),
        })
        .collect()
}

/// Every fixed disk but those `ignore` names, judged by the fullest.
pub fn check(ignore: &[String], disks: &Disks) -> Outcome {
    judge(fixed(all(disks), ignore))
}

/// The fixed disks, as this check counts them, for the machine's inventory
/// (D-85).
pub fn inventory(disks: &Disks) -> Vec<common::inventory::Volume> {
    fixed(all(disks), &[])
        .into_iter()
        .map(|v| common::inventory::Volume {
            mount: v.mount,
            file_system: v.file_system,
            total_mb: v.total / (1024 * 1024),
        })
        .collect()
}

/// Is `mount` one of `ignore`? Each is a mount point or a pattern (`/mnt/*`);
/// `E:` names `E:\` too.
fn ignored(mount: &str, ignore: &[String]) -> bool {
    let bare = mount.trim_end_matches(['/', '\\']);
    ignore
        .iter()
        .any(|pattern| files::matches(pattern, mount) || files::matches(pattern, bare))
}

/// The volumes that are fixed disks of this machine, each once: no
/// removable or read-only ones, none of [`NOT_FIXED`]; of several mounts of
/// one device (bind mounts, subvolumes, container layers) the one with the
/// shortest path; and none of those `ignore` names.
fn fixed(mut all: Vec<Volume>, ignore: &[String]) -> Vec<Volume> {
    all.retain(|v| {
        v.total > 0 && !v.removable && !v.read_only && !NOT_FIXED.contains(&v.file_system.as_str())
    });
    all.sort_by_key(|v| v.mount.len());
    let mut kept: Vec<Volume> = Vec::new();
    for volume in all {
        let again = kept
            .iter()
            .any(|k| k.device == volume.device && k.total == volume.total);
        if !again {
            kept.push(volume);
        }
    }
    // Last: a disk left out stays out, also where it is mounted a second time.
    kept.retain(|v| !ignored(&v.mount, ignore));
    kept
}

fn judge(mut volumes: Vec<Volume>) -> Outcome {
    volumes.sort_by(|a, b| b.usage().total_cmp(&a.usage()));
    let Some(fullest) = volumes.first() else {
        return Outcome::down("No fixed disk found");
    };
    let metrics = vec![
        Metric::new(metric::USAGE, fullest.usage(), metric::PERCENT),
        Metric::new(
            metric::FREE,
            fullest.free as f64 / BYTES_PER_GB,
            metric::GIGABYTES,
        ),
        Metric::new(metric::VOLUMES, volumes.len() as f64, ""),
    ];
    let mut named: Vec<String> = volumes
        .iter()
        .take(DISKS_NAMED)
        .map(|v| {
            format!(
                "{} {:.1}% ({:.1} GB free)",
                v.mount,
                v.usage(),
                v.free as f64 / BYTES_PER_GB
            )
        })
        .collect();
    if volumes.len() > DISKS_NAMED {
        named.push(format!("and {} more", volumes.len() - DISKS_NAMED));
    }
    Outcome::up(named.join(", ")).with_metrics(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    const GB: u64 = 1024 * 1024 * 1024;

    fn volume(mount: &str, device: &str, file_system: &str, total_gb: u64, free_gb: u64) -> Volume {
        Volume {
            mount: mount.into(),
            device: device.into(),
            file_system: file_system.into(),
            total: total_gb * GB,
            free: free_gb * GB,
            removable: false,
            read_only: false,
        }
    }

    fn mounts(volumes: &[Volume]) -> Vec<&str> {
        volumes.iter().map(|v| v.mount.as_str()).collect()
    }

    #[test]
    fn the_fullest_disk_is_the_headline_and_every_disk_is_named() {
        let outcome = judge(vec![
            volume("C:\\", "C:", "ntfs", 100, 60),
            volume("D:\\", "D:", "ntfs", 1000, 80),
            volume("E:\\", "E:", "ntfs", 50, 50),
        ]);
        assert_eq!(outcome.status, Health::Up);
        assert_eq!(
            outcome.message,
            "D:\\ 92.0% (80.0 GB free), C:\\ 40.0% (60.0 GB free), E:\\ 0.0% (50.0 GB free)"
        );
        let values: Vec<_> = outcome
            .metrics
            .iter()
            .map(|m| (m.key.as_str(), m.value))
            .collect();
        assert_eq!(
            values,
            [
                (metric::USAGE, 92.0),
                (metric::FREE, 80.0),
                (metric::VOLUMES, 3.0)
            ]
        );

        let none = judge(vec![]);
        assert_eq!(
            (none.status, none.message.as_str()),
            (Health::Down, "No fixed disk found")
        );
        let many: Vec<_> = (0..9)
            .map(|i| volume(&format!("/data{i}"), &format!("sd{i}"), "ext4", 10, 5))
            .collect();
        assert!(judge(many).message.ends_with(", and 3 more"));
    }

    #[test]
    fn only_fixed_disks_count_and_each_once() {
        let mut usb = volume("/media/usb", "/dev/sdc1", "vfat", 32, 1);
        usb.removable = true;
        let mut installer = volume("/mnt/cd", "/dev/sr0", "ext4", 4, 0);
        installer.read_only = true;
        let all = vec![
            volume("/", "/dev/sda2", "ext4", 100, 40),
            volume("/home", "/dev/sda3", "ext4", 400, 100),
            // A snap: a read-only image, always full.
            volume("/snap/core/1", "/dev/loop0", "squashfs", 1, 0),
            volume("/run", "tmpfs", "tmpfs", 2, 2),
            volume("/mnt/nas", "nas:/vol", "nfs4", 8000, 10),
            usb,
            installer,
            // The same disk mounted again, and a container's layers.
            volume("/var/lib/data", "/dev/sda3", "ext4", 400, 100),
            volume(
                "/var/lib/docker/overlay2/a/merged",
                "overlay",
                "overlay",
                100,
                40,
            ),
            volume(
                "/var/lib/docker/overlay2/b/merged",
                "overlay",
                "overlay",
                100,
                40,
            ),
            volume("/boot/empty", "/dev/sda9", "ext4", 0, 0),
        ];
        assert_eq!(
            mounts(&fixed(all.clone(), &[])),
            ["/", "/home", "/var/lib/docker/overlay2/a/merged"]
        );
        let ignore = ["/var/lib/docker/*".to_string(), "/home".to_string()];
        assert_eq!(mounts(&fixed(all, &ignore)), ["/"]);
    }

    #[test]
    fn a_disk_is_ignored_by_its_mount_point_however_it_is_written() {
        let all = vec![
            volume("C:\\", "C:", "ntfs", 100, 60),
            volume("E:\\", "E:", "ntfs", 100, 1),
        ];
        for written in ["E:", "e:\\", "E:\\"] {
            assert_eq!(
                mounts(&fixed(all.clone(), &[written.to_string()])),
                ["C:\\"],
                "{written}"
            );
        }
    }

    #[test]
    fn this_machines_disks_are_read() {
        let outcome = check(&[], &Disks::new_with_refreshed_list());
        if outcome.status == Health::Up {
            assert_eq!(outcome.metrics[0].key, metric::USAGE);
            assert!((0.0..=100.0).contains(&outcome.metrics[0].value));
            assert!(outcome.metrics[2].value >= 1.0);
        }
        let nothing = check(&["*".to_string()], &Disks::new_with_refreshed_list());
        assert_eq!(nothing.status, Health::Down, "everything ignored");
    }
}
