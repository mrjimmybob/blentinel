//! The `smart` check (M5-16, D-93): are the drives of the probe's machine
//! healthy, by what they say of themselves (SMART)?
//!
//! The probe sends no command to a drive: that takes rights it doesn't have
//! and shouldn't (D-84), and a tool it may not rely on (FP-2). It asks the
//! machine's own system, which already reads its drives: on Linux the disk
//! service (udisks2) over D-Bus, on Windows its storage management. Both
//! answer a program without rights. Read once in ten minutes, whatever the
//! check's interval: a drive's health doesn't change faster, and the
//! systems themselves read it no more often.
//!
//! A drive that says it is failing is Down. Warning signs (bad sectors, an
//! attribute past its limit, a worn-out SSD) are counted for the hub, which
//! makes a Warning of them (the probe says Up or Down only).

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use common::checks::{Metric, metric};

use super::Outcome;
use super::files::matches;

/// How long a reading stands, and a failed one.
const READ_EVERY: Duration = Duration::from_secs(10 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// A wear of this many percent and more is a warning sign: the drive has
/// written what it was built to.
const WORN_OUT: f64 = 100.0;

/// What a drive's health is.
#[derive(Debug, Clone, PartialEq)]
enum State {
    Healthy,
    /// A sign of trouble: which.
    Warning(String),
    /// The drive, or its system, says it is failing: how.
    Failing(String),
    /// It reports SMART, but its health can't be told now: why.
    Unknown(String),
}

/// A drive that reports its health.
#[derive(Debug, Clone, PartialEq)]
struct Drive {
    /// Its model.
    name: String,
    serial: String,
    state: State,
    /// Degrees Celsius.
    temperature: Option<f64>,
    /// Percent of its rated life used.
    wear: Option<f64>,
    bad_sectors: Option<u64>,
}

impl Drive {
    /// As a message names it: the model, with the serial number that tells
    /// which of several to pull out.
    fn label(&self) -> String {
        match (self.name.is_empty(), self.serial.is_empty()) {
            (false, false) => format!("{} (serial {})", self.name, self.serial),
            (false, true) => self.name.clone(),
            (true, false) => format!("The disk with serial {}", self.serial),
            (true, true) => "A disk".to_string(),
        }
    }
}

fn disks(count: usize) -> String {
    match count {
        1 => "1 disk".to_string(),
        count => format!("{count} disks"),
    }
}

/// A drive's name with one space between its words: drives pad their names
/// (`WDC  WDS200T2B0B`), and a name copied from a page has lost the padding.
#[cfg(any(target_os = "linux", windows, test))]
fn tidy(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(any(target_os = "linux", test))]
fn plural(count: u64, one: &str, many: &str) -> String {
    match count {
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    }
}

/// What the check says of `drives`, without those `ignore` names by model
/// or serial number.
fn describe(drives: &[Drive], ignore: &[String]) -> Outcome {
    let left_out = |drive: &Drive| {
        ignore
            .iter()
            .any(|pattern| matches(pattern, &drive.name) || matches(pattern, &drive.serial))
    };
    let counted: Vec<&Drive> = drives.iter().filter(|drive| !left_out(drive)).collect();
    if counted.is_empty() {
        return Outcome::down(if drives.is_empty() {
            "No disk of this machine reports its health (the disks of a virtual machine have \
             none to report)"
        } else {
            "Every disk that reports its health is left out of this check"
        });
    }

    let mut troubles = Vec::new();
    let (mut failing, mut warnings, mut healthy) = (0u32, 0u32, 0usize);
    // The worst first.
    for drive in &counted {
        if let State::Failing(how) = &drive.state {
            failing += 1;
            troubles.push(format!("{}: {how}", drive.label()));
        }
    }
    for drive in &counted {
        if let State::Warning(sign) = &drive.state {
            warnings += 1;
            troubles.push(format!("{}: {sign}", drive.label()));
        }
    }
    for drive in &counted {
        match &drive.state {
            State::Unknown(why) => troubles.push(format!("{}: {why}", drive.label())),
            State::Healthy => healthy += 1,
            _ => {}
        }
    }

    let hottest = counted
        .iter()
        .filter_map(|drive| drive.temperature)
        .reduce(f64::max);
    let most_worn = counted
        .iter()
        .filter_map(|drive| drive.wear)
        .reduce(f64::max);
    let bad_sectors = counted
        .iter()
        .filter_map(|drive| drive.bad_sectors)
        .reduce(u64::saturating_add);
    let mut metrics = vec![
        Metric::new(metric::FAILING, f64::from(failing), ""),
        Metric::new(metric::WARNINGS, f64::from(warnings), ""),
        Metric::new(metric::DRIVES, counted.len() as f64, ""),
    ];
    metrics.extend(hottest.map(|t| Metric::new(metric::TEMPERATURE, t, metric::CELSIUS)));
    metrics.extend(most_worn.map(|w| Metric::new(metric::WEAR, w, metric::PERCENT)));
    metrics.extend(bad_sectors.map(|b| Metric::new(metric::BAD_SECTORS, b as f64, "")));

    // Nothing is known of any of them: the check can't say they are fine.
    if failing == 0 && warnings == 0 && healthy == 0 {
        return Outcome::down(format!(
            "No disk's health can be told: {}",
            troubles.join("; ")
        ))
        .with_metrics(metrics);
    }
    let message = if troubles.is_empty() {
        let mut message = format!("{} healthy", disks(healthy));
        match (hottest, healthy) {
            (Some(hottest), 1) => message.push_str(&format!(", at {hottest:.0} °C")),
            (Some(hottest), _) => message.push_str(&format!(", the hottest at {hottest:.0} °C")),
            (None, _) => {}
        }
        if let Some(worn) = most_worn.filter(|worn| *worn >= 1.0) {
            message.push_str(&match healthy {
                1 => format!(", {worn:.0} % of its life used"),
                _ => format!(", the most worn at {worn:.0} % of its life"),
            });
        }
        message
    } else {
        if healthy > 0 {
            troubles.push(format!(
                "{} healthy",
                disks(healthy).replacen(' ', " other ", 1)
            ));
        }
        troubles.join("; ")
    };
    let outcome = if failing > 0 {
        Outcome::down(message)
    } else {
        Outcome::up(message)
    };
    outcome.with_metrics(metrics)
}

/// The machine's drives that report their health, or why they can't be read.
type Reading = Result<Vec<Drive>, String>;

static LAST: LazyLock<tokio::sync::Mutex<Option<(Instant, Reading)>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

pub async fn check(ignore: &[String]) -> Outcome {
    // One reading at a time: a second check waits for the first one's.
    let mut last = LAST.lock().await;
    let reading = match &*last {
        Some((at, Ok(drives))) if at.elapsed() < READ_EVERY => Ok(drives.clone()),
        Some((at, Err(why))) if at.elapsed() < RETRY_AFTER => Err(why.clone()),
        _ => {
            let reading = platform::drives().await;
            *last = Some((Instant::now(), reading.clone()));
            reading
        }
    };
    match reading {
        Ok(drives) => describe(&drives, ignore),
        Err(why) => Outcome::down(why),
    }
}

#[cfg(any(target_os = "linux", test))]
const KELVIN: f64 = 273.15;

/// A temperature in kelvin as degrees Celsius, to one decimal; `None` for
/// "not known" (zero).
#[cfg(any(target_os = "linux", test))]
fn celsius(kelvin: f64) -> Option<f64> {
    (kelvin > 0.0).then(|| ((kelvin - KELVIN) * 10.0).round() / 10.0)
}

/// A SATA drive as udisks2 has it (`org.freedesktop.UDisks2.Drive.Ata`).
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Default, PartialEq)]
struct Ata {
    supported: bool,
    enabled: bool,
    /// The drive's own verdict: it expects to fail.
    failing: bool,
    /// Reallocated and pending sectors; negative: not known.
    bad_sectors: i64,
    /// Attributes past their limit now; negative: not known.
    attributes_failing: i32,
    kelvin: f64,
    /// When udisks2 last read the drive, in seconds since 1970; 0: never.
    updated: u64,
}

/// `None`: the drive has no SMART (a DVD drive, a virtual disk, a stick).
#[cfg(any(target_os = "linux", test))]
fn from_ata(name: &str, serial: &str, ata: &Ata) -> Option<Drive> {
    if !ata.supported {
        return None;
    }
    let read = ata.enabled && ata.updated != 0;
    let bad_sectors = u64::try_from(ata.bad_sectors).ok().filter(|_| read);
    let state = if !ata.enabled {
        State::Unknown("SMART is turned off on it".into())
    } else if ata.updated == 0 {
        State::Unknown("the system hasn't read its SMART data yet".into())
    } else if ata.failing {
        State::Failing("the drive says it is failing (SMART)".into())
    } else if let Some(bad) = bad_sectors.filter(|bad| *bad > 0) {
        State::Warning(plural(bad, "bad sector", "bad sectors"))
    } else if ata.attributes_failing > 0 {
        State::Warning(plural(
            ata.attributes_failing as u64,
            "SMART attribute is past its limit",
            "SMART attributes are past their limits",
        ))
    } else {
        State::Healthy
    };
    Some(Drive {
        name: tidy(name),
        serial: serial.trim().to_string(),
        state,
        temperature: celsius(ata.kelvin).filter(|_| read),
        wear: None,
        bad_sectors,
    })
}

/// An NVMe drive as udisks2 has it (`org.freedesktop.UDisks2.NVMe.Controller`).
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Default, PartialEq)]
struct Nvme {
    /// The drive's critical warnings, in udisks2's words: `spare`,
    /// `temperature`, `degraded`, `readonly`, `volatile_mem`, `pmr_readonly`.
    warnings: Vec<String>,
    kelvin: f64,
    updated: u64,
    /// Percent of its rated life used; can pass 100.
    percent_used: Option<u8>,
}

#[cfg(any(target_os = "linux", test))]
fn from_nvme(name: &str, serial: &str, nvme: &Nvme) -> Drive {
    let read = nvme.updated != 0;
    let wear = nvme.percent_used.map(f64::from).filter(|_| read);
    let serious: Vec<&str> = nvme
        .warnings
        .iter()
        .filter(|warning| warning.as_str() != "temperature")
        .map(|warning| match warning.as_str() {
            "spare" => "its spare space is running out",
            "degraded" => "its reliability is degraded",
            "readonly" => "it has gone read-only",
            "volatile_mem" => "its memory backup has failed",
            "pmr_readonly" => "its persistent memory is read-only",
            other => other,
        })
        .collect();
    let state = if !read {
        State::Unknown("the system hasn't read its health yet".into())
    } else if !serious.is_empty() {
        State::Failing(format!("the drive says {}", serious.join(", ")))
    } else if !nvme.warnings.is_empty() {
        State::Warning("the drive says it is too hot".into())
    } else if let Some(worn) = wear.filter(|worn| *worn >= WORN_OUT) {
        State::Warning(format!("it has used {worn:.0} % of its rated life"))
    } else {
        State::Healthy
    };
    Drive {
        name: tidy(name),
        serial: serial.trim().to_string(),
        state,
        temperature: celsius(nvme.kelvin).filter(|_| read),
        wear,
        bad_sectors: None,
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::collections::HashMap;

    use zbus::zvariant::{OwnedValue, Value};

    use super::{Ata, Drive, Nvme, Reading, from_ata, from_nvme};

    const SERVICE: &str = "org.freedesktop.UDisks2";
    const ROOT: &str = "/org/freedesktop/UDisks2";
    const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
    const ATA: &str = "org.freedesktop.UDisks2.Drive.Ata";
    const NVME: &str = "org.freedesktop.UDisks2.NVMe.Controller";

    type Props = HashMap<String, OwnedValue>;

    fn number(props: &Props, key: &str) -> Option<f64> {
        Some(match &**props.get(key)? {
            Value::U8(n) => f64::from(*n),
            Value::I16(n) => f64::from(*n),
            Value::U16(n) => f64::from(*n),
            Value::I32(n) => f64::from(*n),
            Value::U32(n) => f64::from(*n),
            Value::I64(n) => *n as f64,
            Value::U64(n) => *n as f64,
            Value::F64(n) => *n,
            _ => return None,
        })
    }

    fn flag(props: &Props, key: &str) -> bool {
        matches!(
            props.get(key).map(|value| &**value),
            Some(Value::Bool(true))
        )
    }

    fn text(props: &Props, key: &str) -> String {
        match props.get(key).map(|value| &**value) {
            Some(Value::Str(text)) => text.to_string(),
            _ => String::new(),
        }
    }

    fn words(props: &Props, key: &str) -> Vec<String> {
        match props.get(key).map(|value| &**value) {
            Some(Value::Array(array)) => array
                .iter()
                .filter_map(|word| match word {
                    Value::Str(word) => Some(word.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// What udisks2 knows of every drive: read from what it holds, no drive
    /// is asked or woken.
    pub async fn drives() -> Reading {
        let connection = zbus::Connection::system()
            .await
            .map_err(|_| "The probe isn't let onto the system bus".to_string())?;
        let unreadable = |e: String| format!("The disk service (udisks2) can't be read: {e}");
        let manager = zbus::fdo::ObjectManagerProxy::builder(&connection)
            .destination(SERVICE)
            .and_then(|builder| builder.path(ROOT))
            .map_err(|e| unreadable(e.to_string()))?
            .build()
            .await
            .map_err(|e| unreadable(e.to_string()))?;
        let objects = manager.get_managed_objects().await.map_err(|e| match e {
            zbus::fdo::Error::ServiceUnknown(_) => {
                "This machine doesn't run the disk service (udisks2) that the probe reads the \
                 disks' health from"
                    .to_string()
            }
            e => unreadable(e.to_string()),
        })?;

        let mut drives = Vec::new();
        for (path, interfaces) in &objects {
            let of = |name: &str| {
                interfaces
                    .iter()
                    .find(|(interface, _)| interface.as_str() == name)
                    .map(|(_, props)| props)
            };
            let Some(drive) = of(DRIVE) else {
                continue;
            };
            let (model, serial) = (text(drive, "Model"), text(drive, "Serial"));
            if let Some(nvme) = of(NVME) {
                let nvme = Nvme {
                    warnings: words(nvme, "SmartCriticalWarning"),
                    kelvin: number(nvme, "SmartTemperature").unwrap_or(0.0),
                    updated: number(nvme, "SmartUpdated").unwrap_or(0.0) as u64,
                    percent_used: percent_used(&connection, path.as_str()).await,
                };
                drives.push(from_nvme(&model, &serial, &nvme));
            } else if let Some(ata) = of(ATA) {
                let ata = Ata {
                    supported: flag(ata, "SmartSupported"),
                    enabled: flag(ata, "SmartEnabled"),
                    failing: flag(ata, "SmartFailing"),
                    bad_sectors: number(ata, "SmartNumBadSectors").unwrap_or(-1.0) as i64,
                    attributes_failing: number(ata, "SmartNumAttributesFailing").unwrap_or(-1.0)
                        as i32,
                    kelvin: number(ata, "SmartTemperature").unwrap_or(0.0),
                    updated: number(ata, "SmartUpdated").unwrap_or(0.0) as u64,
                };
                drives.extend(from_ata(&model, &serial, &ata));
            }
        }
        drives.sort_by(|a: &Drive, b: &Drive| (&a.name, &a.serial).cmp(&(&b.name, &b.serial)));
        Ok(drives)
    }

    /// How much of its life an NVMe drive has used, from the health log
    /// udisks2 holds of it. `None` where it doesn't say.
    async fn percent_used(connection: &zbus::Connection, path: &str) -> Option<u8> {
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        let answer = connection
            .call_method(
                Some(SERVICE),
                path,
                Some(NVME),
                "SmartGetAttributes",
                &(options,),
            )
            .await
            .ok()?;
        let attributes: Props = answer.body().deserialize().ok()?;
        number(&attributes, "percent_used").map(|used| used.clamp(0.0, 255.0) as u8)
    }
}

/// A physical disk as Windows' storage management has it
/// (`MSFT_PhysicalDisk`), with its reliability counters where the probe
/// may read them (`MSFT_StorageReliabilityCounter`: the system's service
/// may, an ordinary user may not).
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Default, PartialEq)]
struct Physical {
    name: String,
    serial: String,
    /// How it is attached, by Windows' number: 11 SATA, 17 NVMe, 7 USB, …
    bus: u16,
    /// 0 healthy, 1 warning, 2 unhealthy; anything else: not known.
    health: Option<u16>,
    /// What state it is in, by Windows' numbers: 2 is OK.
    operational: Vec<u16>,
    /// Degrees Celsius.
    temperature: Option<u64>,
    /// Percent of its rated life used.
    wear: Option<u64>,
}

/// `None`: not a drive of its own (a virtual disk).
#[cfg(any(windows, test))]
fn from_windows(disk: &Physical) -> Option<Drive> {
    /// A virtual disk, and one that is a file.
    const VIRTUAL_BUSES: [u16; 2] = [14, 15];
    const OK: u16 = 2;
    const PREDICTIVE_FAILURE: u16 = 5;
    if VIRTUAL_BUSES.contains(&disk.bus) {
        return None;
    }
    let states: Vec<String> = disk
        .operational
        .iter()
        .filter(|state| **state != OK)
        .map(|state| match state {
            3 => "degraded".to_string(),
            4 => "stressed".to_string(),
            5 => "a failure is predicted".to_string(),
            6 => "error".to_string(),
            7 => "an error it can't recover from".to_string(),
            10 => "stopped".to_string(),
            12 => "no contact".to_string(),
            13 => "lost communication".to_string(),
            other => format!("state {other}"),
        })
        .collect();
    let said = |verdict: &str| match states.is_empty() {
        true => verdict.to_string(),
        false => format!("{verdict} ({})", states.join(", ")),
    };
    let wear = disk
        .wear
        .filter(|wear| *wear <= 255)
        .map(|wear| wear as f64);
    let state = if disk.operational.contains(&PREDICTIVE_FAILURE) {
        State::Failing("the drive says it is failing (SMART)".into())
    } else {
        match disk.health {
            Some(2) => State::Failing(said("Windows says it is unhealthy")),
            Some(1) => State::Warning(said("Windows says it needs attention")),
            Some(0) => match wear.filter(|worn| *worn >= WORN_OUT) {
                Some(worn) => State::Warning(format!("it has used {worn:.0} % of its rated life")),
                None => State::Healthy,
            },
            _ => State::Unknown("Windows doesn't know its health".into()),
        }
    };
    Some(Drive {
        name: tidy(&disk.name),
        serial: disk.serial.trim().trim_end_matches('.').to_string(),
        state,
        temperature: disk
            .temperature
            .filter(|t| (1..=150).contains(t))
            .map(|t| t as f64),
        wear,
        bad_sectors: None,
    })
}

/// The numbers in a text Windows gave: `2`, or `2; 5` for a list.
#[cfg(any(windows, test))]
fn numbers(text: &str) -> Vec<u64> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter_map(|number| number.parse().ok())
        .collect()
}

#[cfg(windows)]
mod platform {
    use super::{Drive, Physical, Reading, from_windows, numbers};
    use crate::checks::wmi::{self, Namespace, Row};

    /// Where Windows keeps what it knows of its storage (Windows 8 and
    /// Server 2012 on): what `Get-PhysicalDisk` shows.
    const NAMESPACE: &str = "ROOT\\Microsoft\\Windows\\Storage";
    const DISK_FIELDS: [&str; 6] = [
        "DeviceId",
        "FriendlyName",
        "SerialNumber",
        "BusType",
        "HealthStatus",
        "OperationalStatus",
    ];
    const COUNTER_FIELDS: [&str; 3] = ["DeviceId", "Temperature", "Wear"];

    pub async fn drives() -> Reading {
        wmi::read(|| {
            ask().map_err(|e| format!("Windows' storage management can't be read: {}", e.message()))
        })
        .await
        .unwrap_or_else(|| Err("Reading Windows' storage management failed".to_string()))
    }

    fn ask() -> windows::core::Result<Vec<Drive>> {
        let storage = Namespace::open(NAMESPACE)?;
        let disks = storage.rows("MSFT_PhysicalDisk", &DISK_FIELDS)?;
        // Only an administrator, as the installed probe is, may read
        // these: without them the health stands, the numbers are missing.
        let counters = storage
            .rows("MSFT_StorageReliabilityCounter", &COUNTER_FIELDS)
            .inspect_err(|e| {
                tracing::debug!("No reliability counters: {}", e.message());
            })
            .unwrap_or_default();

        let first = |row: &Row, field: &str| {
            row.get(field)
                .and_then(|text| numbers(text).first().copied())
        };
        let mut drives: Vec<Drive> = disks
            .iter()
            .filter_map(|disk| {
                let id = disk.get("DeviceId");
                let counter = counters
                    .iter()
                    .find(|counter| id.is_some() && counter.get("DeviceId") == id);
                from_windows(&Physical {
                    name: disk.get("FriendlyName").cloned().unwrap_or_default(),
                    serial: disk.get("SerialNumber").cloned().unwrap_or_default(),
                    bus: first(disk, "BusType").unwrap_or(0) as u16,
                    health: first(disk, "HealthStatus").map(|health| health as u16),
                    operational: disk
                        .get("OperationalStatus")
                        .map(|text| numbers(text).into_iter().map(|n| n as u16).collect())
                        .unwrap_or_default(),
                    temperature: counter.and_then(|counter| first(counter, "Temperature")),
                    wear: counter.and_then(|counter| first(counter, "Wear")),
                })
            })
            .collect();
        drives.sort_by(|a, b| (&a.name, &a.serial).cmp(&(&b.name, &b.serial)));
        Ok(drives)
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::Reading;

    pub async fn drives() -> Reading {
        Err("Disk health checks work on Windows and Linux only".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;

    fn drive(name: &str, serial: &str, state: State, temperature: Option<f64>) -> Drive {
        Drive {
            name: name.into(),
            serial: serial.into(),
            state,
            temperature,
            wear: None,
            bad_sectors: None,
        }
    }

    fn value(outcome: &Outcome, key: &str) -> Option<f64> {
        outcome
            .metrics
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.value)
    }

    #[test]
    fn healthy_drives_are_counted_with_the_hottest_and_the_most_worn() {
        let one = describe(&[drive("WD Blue", "A1", State::Healthy, Some(34.2))], &[]);
        assert_eq!(one.status, Health::Up);
        assert_eq!(one.message, "1 disk healthy, at 34 °C");

        let mut worn = drive("Samsung 980", "N1", State::Healthy, Some(47.0));
        worn.wear = Some(12.0);
        let mut old = drive("ST4000", "S1", State::Healthy, Some(31.0));
        old.bad_sectors = Some(0);
        let all = describe(
            &[drive("WD Blue", "A1", State::Healthy, None), worn, old],
            &[],
        );
        assert_eq!(all.status, Health::Up);
        assert_eq!(
            all.message,
            "3 disks healthy, the hottest at 47 °C, the most worn at 12 % of its life"
        );
        let keys: Vec<&str> = all.metrics.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                metric::FAILING,
                metric::WARNINGS,
                metric::DRIVES,
                metric::TEMPERATURE,
                metric::WEAR,
                metric::BAD_SECTORS
            ]
        );
        assert_eq!(value(&all, metric::FAILING), Some(0.0));
        assert_eq!(value(&all, metric::DRIVES), Some(3.0));
        assert_eq!(value(&all, metric::TEMPERATURE), Some(47.0));
        assert_eq!(value(&all, metric::WEAR), Some(12.0));
        assert_eq!(value(&all, metric::BAD_SECTORS), Some(0.0));

        // What no drive says isn't reported.
        let bare = describe(&[drive("WD Blue", "", State::Healthy, None)], &[]);
        assert_eq!(bare.message, "1 disk healthy");
        assert_eq!(value(&bare, metric::TEMPERATURE), None);
        assert_eq!(value(&bare, metric::WEAR), None);
    }

    #[test]
    fn a_failing_drive_is_down_and_a_warning_sign_is_counted_for_the_hub() {
        let failing = State::Failing("the drive says it is failing (SMART)".into());
        let sectors = State::Warning("8 bad sectors".into());
        let machine = [
            drive("WD Blue", "A1", State::Healthy, Some(30.0)),
            drive("ST4000DM004", "ZFN0", sectors.clone(), Some(41.0)),
            drive("ST3000DM007", "W1F2", failing, Some(38.0)),
            drive("Samsung 980", "N1", State::Healthy, None),
        ];
        let down = describe(&machine, &[]);
        assert_eq!(down.status, Health::Down);
        // The worst first, each with the serial number to find it by.
        assert_eq!(
            down.message,
            "ST3000DM007 (serial W1F2): the drive says it is failing (SMART); \
             ST4000DM004 (serial ZFN0): 8 bad sectors; 2 other disks healthy"
        );
        assert_eq!(value(&down, metric::FAILING), Some(1.0));
        assert_eq!(value(&down, metric::WARNINGS), Some(1.0));
        assert_eq!(value(&down, metric::DRIVES), Some(4.0));

        // A warning sign alone: Up for the probe, with the count the hub
        // makes a Warning of.
        let warned = describe(&machine[..2], &[]);
        assert_eq!(warned.status, Health::Up);
        assert_eq!(
            warned.message,
            "ST4000DM004 (serial ZFN0): 8 bad sectors; 1 other disk healthy"
        );
        assert_eq!(value(&warned, metric::WARNINGS), Some(1.0));
        let alone = describe(&machine[1..2], &[]);
        assert_eq!(alone.message, "ST4000DM004 (serial ZFN0): 8 bad sectors");
    }

    #[test]
    fn drives_are_left_out_by_model_or_serial_and_none_left_is_down() {
        let failing = State::Failing("the drive says it is failing (SMART)".into());
        let machine = [
            drive("WD Blue", "A1", State::Healthy, None),
            drive("ST3000DM007", "W1F2", failing, None),
        ];
        for pattern in ["st3000*", "W1F2", "ST3000DM007"] {
            let without = describe(&machine, &[pattern.to_string()]);
            assert_eq!(without.status, Health::Up, "{pattern}");
            assert_eq!(without.message, "1 disk healthy", "{pattern}");
            assert_eq!(value(&without, metric::DRIVES), Some(1.0));
        }
        let none = describe(&machine, &["*".to_string()]);
        assert_eq!(none.status, Health::Down);
        assert_eq!(
            none.message,
            "Every disk that reports its health is left out of this check"
        );
        let virtual_machine = describe(&[], &[]);
        assert_eq!(virtual_machine.status, Health::Down);
        assert!(
            virtual_machine
                .message
                .starts_with("No disk of this machine reports its health"),
            "{}",
            virtual_machine.message
        );

        // A drive whose health can't be told is said beside the others, and
        // alone it leaves nothing to say Up on.
        let off = drive(
            "Old WD",
            "X9",
            State::Unknown("SMART is turned off on it".into()),
            None,
        );
        let mixed = describe(&[machine[0].clone(), off.clone()], &[]);
        assert_eq!(mixed.status, Health::Up);
        assert_eq!(
            mixed.message,
            "Old WD (serial X9): SMART is turned off on it; 1 other disk healthy"
        );
        assert_eq!(value(&mixed, metric::WARNINGS), Some(0.0));
        let only = describe(&[off], &[]);
        assert_eq!(only.status, Health::Down);
        assert_eq!(
            only.message,
            "No disk's health can be told: Old WD (serial X9): SMART is turned off on it"
        );
    }

    #[test]
    fn a_sata_drive_is_read_as_udisks_has_it() {
        // The Fedora machine's SSD, as its udisks2 gave it.
        let ssd = Ata {
            supported: true,
            enabled: true,
            failing: false,
            bad_sectors: 0,
            attributes_failing: 0,
            kelvin: 301.0,
            updated: 1_791_846_773,
        };
        let healthy = from_ata("WDC  WDS200T2B0B-00YS70 ", "213167801692", &ssd).unwrap();
        assert_eq!(
            healthy,
            Drive {
                name: "WDC WDS200T2B0B-00YS70".into(),
                serial: "213167801692".into(),
                state: State::Healthy,
                temperature: Some(27.9),
                wear: None,
                bad_sectors: Some(0),
            }
        );
        let state = |change: fn(&mut Ata)| {
            let mut ata = ssd.clone();
            change(&mut ata);
            from_ata("d", "s", &ata).unwrap().state
        };
        assert_eq!(
            state(|a| a.failing = true),
            State::Failing("the drive says it is failing (SMART)".into())
        );
        // Its own verdict counts before the signs.
        assert!(matches!(
            state(|a| (a.failing, a.bad_sectors) = (true, 8)),
            State::Failing(_)
        ));
        assert_eq!(
            state(|a| a.bad_sectors = 8),
            State::Warning("8 bad sectors".into())
        );
        assert_eq!(
            state(|a| a.bad_sectors = 1),
            State::Warning("1 bad sector".into())
        );
        assert_eq!(
            state(|a| a.attributes_failing = 2),
            State::Warning("2 SMART attributes are past their limits".into())
        );
        // "Not known" (-1) is no sign.
        assert_eq!(
            state(|a| (a.bad_sectors, a.attributes_failing) = (-1, -1)),
            State::Healthy
        );
        assert_eq!(
            state(|a| a.enabled = false),
            State::Unknown("SMART is turned off on it".into())
        );
        assert_eq!(
            state(|a| a.updated = 0),
            State::Unknown("the system hasn't read its SMART data yet".into())
        );
        // What was never read has no numbers.
        let unread = from_ata(
            "d",
            "s",
            &Ata {
                updated: 0,
                ..ssd.clone()
            },
        )
        .unwrap();
        assert_eq!((unread.temperature, unread.bad_sectors), (None, None));
        // A DVD drive, a virtual disk: no SMART, not a drive to count.
        assert_eq!(from_ata("QEMU DVD-ROM", "QM00004", &Ata::default()), None);
        assert_eq!(celsius(0.0), None);
    }

    #[test]
    fn an_nvme_drive_is_read_as_udisks_has_it() {
        let nvme = Nvme {
            warnings: vec![],
            kelvin: 318.0,
            updated: 1_791_846_773,
            percent_used: Some(7),
        };
        let healthy = from_nvme("Samsung SSD 980 PRO 1TB", "S5GX", &nvme);
        assert_eq!(healthy.state, State::Healthy);
        assert_eq!(
            (healthy.temperature, healthy.wear, healthy.bad_sectors),
            (Some(44.9), Some(7.0), None)
        );
        let state = |change: fn(&mut Nvme)| {
            let mut nvme = nvme.clone();
            change(&mut nvme);
            from_nvme("d", "s", &nvme).state
        };
        assert_eq!(
            state(|n| n.warnings = vec!["spare".into(), "temperature".into()]),
            State::Failing("the drive says its spare space is running out".into())
        );
        assert_eq!(
            state(|n| n.warnings = vec!["degraded".into(), "readonly".into()]),
            State::Failing(
                "the drive says its reliability is degraded, it has gone read-only".into()
            )
        );
        assert_eq!(
            state(|n| n.warnings = vec!["temperature".into()]),
            State::Warning("the drive says it is too hot".into())
        );
        assert_eq!(
            state(|n| n.percent_used = Some(104)),
            State::Warning("it has used 104 % of its rated life".into())
        );
        assert_eq!(state(|n| n.percent_used = Some(99)), State::Healthy);
        assert_eq!(state(|n| n.percent_used = None), State::Healthy);
        assert_eq!(
            state(|n| n.updated = 0),
            State::Unknown("the system hasn't read its health yet".into())
        );
    }

    #[test]
    fn a_disk_is_read_as_windows_has_it() {
        let disk = Physical {
            name: "ST4000DM004-2CV104".into(),
            serial: "  ZFN0ABCD.".into(),
            bus: 11,
            health: Some(0),
            operational: vec![2],
            temperature: Some(36),
            wear: Some(0),
        };
        let healthy = from_windows(&disk).unwrap();
        assert_eq!(
            healthy,
            Drive {
                name: "ST4000DM004-2CV104".into(),
                serial: "ZFN0ABCD".into(),
                state: State::Healthy,
                temperature: Some(36.0),
                wear: Some(0.0),
                bad_sectors: None,
            }
        );
        let state = |change: fn(&mut Physical)| {
            let mut disk = disk.clone();
            change(&mut disk);
            from_windows(&disk).unwrap().state
        };
        assert_eq!(
            state(|d| d.operational = vec![5]),
            State::Failing("the drive says it is failing (SMART)".into())
        );
        assert_eq!(
            state(|d| (d.health, d.operational) = (Some(2), vec![6, 13])),
            State::Failing("Windows says it is unhealthy (error, lost communication)".into())
        );
        assert_eq!(
            state(|d| d.health = Some(2)),
            State::Failing("Windows says it is unhealthy".into())
        );
        assert_eq!(
            state(|d| (d.health, d.operational) = (Some(1), vec![3])),
            State::Warning("Windows says it needs attention (degraded)".into())
        );
        assert_eq!(
            state(|d| d.wear = Some(100)),
            State::Warning("it has used 100 % of its rated life".into())
        );
        for unknown in [None, Some(5)] {
            let mut disk = disk.clone();
            disk.health = unknown;
            assert_eq!(
                from_windows(&disk).unwrap().state,
                State::Unknown("Windows doesn't know its health".into())
            );
        }
        // Without the counters (an ordinary user): the health, no numbers.
        let plain = from_windows(&Physical {
            temperature: None,
            wear: None,
            ..disk.clone()
        })
        .unwrap();
        assert_eq!(
            (plain.state, plain.temperature, plain.wear),
            (State::Healthy, None, None)
        );
        // A temperature of 0 is "not known"; a virtual disk isn't a drive.
        let cold = from_windows(&Physical {
            temperature: Some(0),
            ..disk.clone()
        });
        assert_eq!(cold.unwrap().temperature, None);
        for bus in [14, 15] {
            assert_eq!(
                from_windows(&Physical {
                    bus,
                    ..disk.clone()
                }),
                None
            );
        }

        assert_eq!(numbers("2"), [2]);
        assert_eq!(numbers("2; 5"), [2, 5]);
        assert_eq!(numbers(""), Vec::<u64>::new());
    }

    /// By hand (`--ignored --nocapture`): what this machine's system says
    /// of its drives. As an administrator on Windows for the temperatures.
    #[tokio::test]
    #[ignore = "reads this machine's drives: run by hand"]
    async fn this_machines_drives() {
        match platform::drives().await {
            Ok(drives) => {
                for drive in &drives {
                    println!("{drive:?}");
                }
                let outcome = describe(&drives, &[]);
                println!("{:?}: {}", outcome.status, outcome.message);
                println!("{:?}", outcome.metrics);
            }
            Err(why) => println!("Down: {why}"),
        }
    }
}
