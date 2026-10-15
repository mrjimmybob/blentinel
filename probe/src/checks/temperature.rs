//! The `temperature` check (M5-16b, D-94): how hot is the probe's machine,
//! by its own sensors (CPU, board, drives)?
//!
//! The probe reads what the machine's system already shows a program
//! without rights: on Linux the kernel's sensor list (`/sys/class/hwmon`),
//! on Windows the firmware's thermal zone. It loads no driver and runs no
//! tool (FP-2), so a sensor that needs a vendor's driver isn't seen: most
//! Windows desktops and servers show none, and a virtual machine has none.
//!
//! Linux's list is read here, not through `sysinfo`: that names a sensor
//! without its chip (`Core 0`) and keeps one of two by the same name, so
//! the second CPU of a server, or the second drive, would go unseen.
//!
//! The hottest sensor's temperature is the check's value; the hub judges it
//! by its limits. A sensor at the critical limit its own hardware gives is
//! Down whatever the hub's limits are.

use std::ops::RangeInclusive;

use common::checks::{Metric, metric};

use super::Outcome;
use super::files::matches;

/// What a working sensor can say, in degrees Celsius. Anything else (0, a
/// negative number, 200) is a sensor that isn't connected.
const SENSIBLE: RangeInclusive<f64> = 1.0..=150.0;
/// How many sensors a message names when none matches.
const NAMED: usize = 8;

/// A temperature sensor and what it reads.
#[derive(Debug, Clone, PartialEq)]
struct Sensor {
    /// As the system names it: `coretemp Package id 0`, `acpitz temp1`.
    name: String,
    celsius: f64,
    /// The limit the hardware itself gives, where it gives one.
    critical: Option<f64>,
}

/// A sensor from what the system says of it. `None`: it reads nothing
/// sensible.
fn sensor(name: &str, celsius: Option<f64>, critical: Option<f64>) -> Option<Sensor> {
    let celsius = celsius?;
    SENSIBLE.contains(&celsius).then(|| Sensor {
        // One space between the words, as a page shows it.
        name: name.split_whitespace().collect::<Vec<_>>().join(" "),
        celsius,
        critical: critical.filter(|limit| SENSIBLE.contains(limit)),
    })
}

fn degrees(celsius: f64) -> String {
    format!("{celsius:.0} {}", metric::CELSIUS)
}

/// What the check says of `sensors`, of which only those `wanted` names
/// count (none: every one).
fn describe(sensors: &[Sensor], wanted: &[String]) -> Outcome {
    if sensors.is_empty() {
        return Outcome::down(if cfg!(windows) {
            "Windows shows no temperature sensor of this machine (it reads only the firmware's \
             own, which most desktops, servers and virtual machines don't have)"
        } else {
            "This machine shows no temperature sensor (a virtual machine has none)"
        });
    }
    let counted: Vec<&Sensor> = sensors
        .iter()
        .filter(|sensor| {
            wanted.is_empty() || wanted.iter().any(|pattern| matches(pattern, &sensor.name))
        })
        .collect();
    // Of two as hot, the first: a CPU's package comes before its cores.
    let Some(hottest) = counted
        .iter()
        .rev()
        .max_by(|a, b| a.celsius.total_cmp(&b.celsius))
    else {
        let mut names: Vec<&str> = sensors.iter().map(|sensor| sensor.name.as_str()).collect();
        names.dedup();
        let more = names.len().saturating_sub(NAMED);
        names.truncate(NAMED);
        return Outcome::down(format!(
            "No sensor of this machine is named {}. It has: {}{}",
            wanted.join(", "),
            names.join(", "),
            if more > 0 {
                format!(", and {more} more")
            } else {
                String::new()
            },
        ));
    };

    let metrics = vec![
        Metric::new(
            metric::TEMPERATURE,
            (hottest.celsius * 10.0).round() / 10.0,
            metric::CELSIUS,
        ),
        Metric::new(metric::SENSORS, counted.len() as f64, ""),
    ];
    // The sensor nearest its own critical limit, if one has reached it.
    let critical = counted
        .iter()
        .filter_map(|sensor| Some((sensor, sensor.critical?)))
        .filter(|(sensor, limit)| sensor.celsius >= *limit)
        .max_by(|a, b| a.0.celsius.total_cmp(&b.0.celsius));
    if let Some((sensor, limit)) = critical {
        return Outcome::down(format!(
            "{} is at {}, its critical limit is {}",
            sensor.name,
            degrees(sensor.celsius),
            degrees(limit),
        ))
        .with_metrics(metrics);
    }
    Outcome::up(match counted.len() {
        1 => format!("{}: {}", hottest.name, degrees(hottest.celsius)),
        count => format!(
            "{} is the hottest of {count} sensors, at {}",
            hottest.name,
            degrees(hottest.celsius),
        ),
    })
    .with_metrics(metrics)
}

/// A file of Linux's sensor lists: one number, in thousandths of a degree.
#[cfg(any(target_os = "linux", test))]
fn millidegrees(file: &std::path::Path) -> Option<f64> {
    let text = std::fs::read_to_string(file).ok()?;
    Some(text.trim().parse::<f64>().ok()? / 1000.0)
}

/// The folders of `root` named `prefix` and a number, in that order.
#[cfg(any(target_os = "linux", test))]
fn numbered(root: &std::path::Path, prefix: &str) -> Vec<std::path::PathBuf> {
    let mut found: Vec<(u32, std::path::PathBuf)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let number = entry
                .file_name()
                .to_str()?
                .strip_prefix(prefix)?
                .parse()
                .ok()?;
            Some((number, entry.path()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

/// The sensors of Linux's hardware monitoring (`/sys/class/hwmon`): each
/// chip's `tempN_input`, named by the chip and the sensor's label
/// (`coretemp Package id 0`), or its number where it has none
/// (`acpitz temp1`), with the chip's own critical limit (`tempN_crit`).
#[cfg(any(target_os = "linux", test))]
fn hwmon(root: &std::path::Path) -> Vec<Sensor> {
    let line = |file: std::path::PathBuf| {
        std::fs::read_to_string(file)
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };
    let mut sensors = Vec::new();
    for chip in numbered(root, "hwmon") {
        let name = line(chip.join("name")).unwrap_or_else(|| "sensor".to_string());
        let mut inputs: Vec<u32> = std::fs::read_dir(&chip)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()?
                    .strip_prefix("temp")?
                    .strip_suffix("_input")?
                    .parse()
                    .ok()
            })
            .collect();
        inputs.sort();
        for input in inputs {
            let file = |what: &str| chip.join(format!("temp{input}_{what}"));
            let label = line(file("label")).unwrap_or_else(|| format!("temp{input}"));
            sensors.extend(sensor(
                &format!("{name} {label}"),
                millidegrees(&file("input")),
                millidegrees(&file("crit")),
            ));
        }
    }
    sensors
}

/// The kernel's thermal zones (`/sys/class/thermal`), for a machine whose
/// only sensor isn't in the list above (some small ARM boards).
#[cfg(any(target_os = "linux", test))]
fn thermal_zones(root: &std::path::Path) -> Vec<Sensor> {
    numbered(root, "thermal_zone")
        .into_iter()
        .filter_map(|zone| {
            let kind = std::fs::read_to_string(zone.join("type")).unwrap_or_default();
            sensor(
                match kind.trim() {
                    "" => "thermal zone",
                    kind => kind,
                },
                millidegrees(&zone.join("temp")),
                None,
            )
        })
        .collect()
}

/// The machine's sensors, as its system shows them now.
#[cfg(target_os = "linux")]
fn read() -> Vec<Sensor> {
    let sensors = hwmon(std::path::Path::new("/sys/class/hwmon"));
    if sensors.is_empty() {
        return thermal_zones(std::path::Path::new("/sys/class/thermal"));
    }
    sensors
}

/// The machine's sensors, as its system shows them now: on Windows the
/// firmware's thermal zone, which the installed probe may read.
#[cfg(not(target_os = "linux"))]
fn read() -> Vec<Sensor> {
    sysinfo::Components::new_with_refreshed_list()
        .list()
        .iter()
        .filter_map(|component| {
            sensor(
                component.label(),
                component.temperature().map(f64::from),
                component.critical().map(f64::from),
            )
        })
        .collect()
}

pub async fn check(wanted: &[String]) -> Outcome {
    // Files on Linux, a query to Windows: neither belongs on the runtime.
    match tokio::task::spawn_blocking(read).await {
        Ok(sensors) => describe(&sensors, wanted),
        Err(error) => Outcome::down(format!("The sensors couldn't be read: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    fn at(name: &str, celsius: f64, critical: Option<f64>) -> Sensor {
        sensor(name, Some(celsius), critical).unwrap()
    }

    /// A laptop's sensors, as Linux names them.
    fn laptop() -> Vec<Sensor> {
        vec![
            at("acpitz temp1", 69.0, None),
            at("pch_skylake temp1", 55.5, None),
            at("coretemp Package id 0", 73.0, Some(100.0)),
            at("coretemp Core 0", 71.0, Some(100.0)),
            at("nvme Composite  WDC PC SN730", 41.9, Some(84.8)),
        ]
    }

    fn value(outcome: &Outcome, key: &str) -> Option<f64> {
        outcome
            .metrics
            .iter()
            .find(|metric| metric.key == key)
            .map(|metric| metric.value)
    }

    #[test]
    fn the_hottest_sensor_is_the_checks_value() {
        let all = describe(&laptop(), &[]);
        assert_eq!(all.status, Health::Up);
        assert_eq!(
            all.message,
            "coretemp Package id 0 is the hottest of 5 sensors, at 73 °C"
        );
        assert_eq!(value(&all, metric::TEMPERATURE), Some(73.0));
        assert_eq!(value(&all, metric::SENSORS), Some(5.0));
        assert_eq!(all.metrics[0].unit, "°C");

        // Of two as hot, the first is named.
        let mut tied = laptop();
        tied[3].celsius = 73.0;
        assert!(
            describe(&tied, &[])
                .message
                .starts_with("coretemp Package id 0 is")
        );
    }

    #[test]
    fn only_the_sensors_named_count() {
        // Whatever the case, with wildcards; a name is kept with one space.
        let drive = describe(&laptop(), &["NVME*".into()]);
        assert_eq!(drive.message, "nvme Composite WDC PC SN730: 42 °C");
        assert_eq!(value(&drive, metric::TEMPERATURE), Some(41.9));
        assert_eq!(value(&drive, metric::SENSORS), Some(1.0));

        let two = describe(&laptop(), &["acpitz*".into(), "pch*".into()]);
        assert_eq!(
            two.message,
            "acpitz temp1 is the hottest of 2 sensors, at 69 °C"
        );

        // None by that name: Down, and the names there are, to pick from.
        let none = describe(&laptop(), &["k10temp*".into()]);
        assert_eq!(none.status, Health::Down);
        assert_eq!(
            none.message,
            "No sensor of this machine is named k10temp*. It has: acpitz temp1, pch_skylake \
             temp1, coretemp Package id 0, coretemp Core 0, nvme Composite WDC PC SN730"
        );
        assert!(none.metrics.is_empty());

        let many: Vec<Sensor> = (0..12)
            .map(|core| at(&format!("coretemp Core {core}"), 50.0, None))
            .collect();
        let none = describe(&many, &["x".into()]);
        assert!(none.message.ends_with("coretemp Core 7, and 4 more"));
    }

    #[test]
    fn a_sensor_at_its_own_critical_limit_is_down() {
        let mut sensors = laptop();
        sensors[4].celsius = 85.0;
        let hot = describe(&sensors, &[]);
        assert_eq!(hot.status, Health::Down);
        assert_eq!(
            hot.message,
            "nvme Composite WDC PC SN730 is at 85 °C, its critical limit is 85 °C"
        );
        // The value is still the hottest sensor's, for the history.
        assert_eq!(value(&hot, metric::TEMPERATURE), Some(85.0));

        // Left out of the check, it doesn't count.
        let cpu = describe(&sensors, &["coretemp*".into()]);
        assert_eq!(cpu.status, Health::Up);
    }

    #[test]
    fn a_sensor_that_says_nothing_sensible_is_not_one() {
        assert_eq!(sensor("acpitz temp1", None, None), None);
        for nonsense in [0.0, -273.2, -0.1, 200.0, f64::NAN] {
            assert_eq!(sensor("it8686 temp3", Some(nonsense), None), None);
        }
        // A limit that makes no sense is no limit.
        let unlimited = at("it8686 temp1", 40.0, Some(0.0));
        assert_eq!(unlimited.critical, None);
        assert_eq!(describe(&[unlimited], &[]).status, Health::Up);
    }

    #[test]
    fn a_machine_without_sensors_is_down() {
        let none = describe(&[], &[]);
        assert_eq!(none.status, Health::Down);
        assert!(none.message.contains("no temperature sensor"));
        assert!(none.metrics.is_empty());
    }

    /// A laptop's `/sys/class/hwmon`, as Fedora 44 showed it, and a second
    /// CPU beside the first.
    #[test]
    fn linux_sensors_are_read_chip_by_chip_and_named_with_their_chip() {
        let root = tempfile::tempdir().unwrap();
        let chip = |number: u32, name: &str, files: &[(&str, &str)]| {
            let folder = root.path().join(format!("hwmon{number}"));
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(folder.join("name"), format!("{name}\n")).unwrap();
            for (file, text) in files {
                std::fs::write(folder.join(file), format!("{text}\n")).unwrap();
            }
        };
        // No temperature: a power supply, a fan.
        chip(0, "AC0", &[]);
        chip(
            4,
            "asus",
            &[("fan1_input", "2600"), ("fan1_label", "cpu_fan")],
        );
        chip(1, "acpitz", &[("temp1_input", "69000")]);
        let cpu = [
            ("temp1_input", "73000"),
            ("temp1_label", "Package id 0"),
            ("temp1_crit", "100000"),
            ("temp1_max", "100000"),
            ("temp2_input", "71000"),
            ("temp2_label", "Core 0"),
            ("temp2_crit", "100000"),
            ("temp10_input", "70500"),
            ("temp10_label", "Core 8"),
        ];
        chip(5, "coretemp", &cpu);
        chip(10, "coretemp", &cpu[..3]);
        // A sensor that isn't connected, and one that can't be read.
        chip(
            6,
            "it8686",
            &[("temp1_input", "-55000"), ("temp2_input", "")],
        );
        std::fs::write(root.path().join("notes"), "not a chip").unwrap();

        let sensors = hwmon(root.path());
        let names: Vec<&str> = sensors.iter().map(|sensor| sensor.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "acpitz temp1",
                "coretemp Package id 0",
                "coretemp Core 0",
                "coretemp Core 8",
                "coretemp Package id 0",
            ]
        );
        assert_eq!(sensors[0], at("acpitz temp1", 69.0, None));
        assert_eq!(sensors[1], at("coretemp Package id 0", 73.0, Some(100.0)));
        assert_eq!(sensors[3], at("coretemp Core 8", 70.5, None));
        // Both CPUs count; of two as hot, the first is named.
        assert_eq!(
            describe(&sensors, &["coretemp*".into()]).message,
            "coretemp Package id 0 is the hottest of 4 sensors, at 73 °C"
        );

        assert_eq!(hwmon(&root.path().join("missing")), []);
    }

    #[test]
    fn a_machine_with_thermal_zones_only_is_read_from_them() {
        let root = tempfile::tempdir().unwrap();
        for (number, kind, temp) in [(1, "gpu-thermal", "41234"), (0, "cpu-thermal", "48312")] {
            let zone = root.path().join(format!("thermal_zone{number}"));
            std::fs::create_dir(&zone).unwrap();
            std::fs::write(zone.join("type"), format!("{kind}\n")).unwrap();
            std::fs::write(zone.join("temp"), format!("{temp}\n")).unwrap();
        }
        std::fs::create_dir(root.path().join("cooling_device0")).unwrap();
        assert_eq!(
            thermal_zones(root.path()),
            [
                at("cpu-thermal", 48.312, None),
                at("gpu-thermal", 41.234, None)
            ]
        );
    }

    /// By hand (`--ignored --nocapture`): what this machine's sensors say.
    #[tokio::test]
    #[ignore = "reads this machine's sensors: run by hand"]
    async fn this_machines_sensors() {
        for sensor in read() {
            println!("{sensor:?}");
        }
        let outcome = check(&[]).await;
        println!("{:?}: {}", outcome.status, outcome.message);
        println!("{:?}", outcome.metrics);
    }
}
