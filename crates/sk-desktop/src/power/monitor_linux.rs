use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Duration;

use ashpd::zbus::{message::Type, Connection, MatchRule, MessageStream, Proxy};
use futures_lite::{stream, StreamExt};

use super::{set_scheme, CpuTimes, Pressure, Scheme};

const PROFILES: [(&str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
    ),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

fn profile(value: &str) -> Scheme {
    match value {
        "power-saver" => Scheme::Saving,
        "performance" => Scheme::Performance,
        _ => Scheme::Balanced,
    }
}

async fn scheme(connection: &Connection) -> Scheme {
    for (name, path) in PROFILES {
        if let Ok(proxy) = Proxy::new(connection, name, path, name).await {
            if let Ok(active) = proxy.get_property::<String>("ActiveProfile").await {
                return profile(&active);
            }
        }
    }
    Scheme::Balanced
}

async fn monitor() -> ashpd::zbus::Result<()> {
    let connection = ashpd::zbus::connection::Builder::system()?
        .method_timeout(Duration::from_secs(3))
        .build()
        .await?;
    let mut changes = Vec::new();
    for (name, path) in PROFILES {
        let properties = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(name)?
            .path(path)?
            .interface("org.freedesktop.DBus.Properties")?
            .member("PropertiesChanged")?
            .arg(0, name)?
            .build();
        let owners = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg(0, name)?
            .build();
        changes.push(stream::or(
            MessageStream::for_match_rule(properties, &connection, Some(8)).await?,
            MessageStream::for_match_rule(owners, &connection, Some(4)).await?,
        ));
    }
    let mut changes = stream::or(changes.pop().unwrap(), changes.pop().unwrap());
    set_scheme(scheme(&connection).await);
    while let Some(change) = changes.next().await {
        change?;
        set_scheme(scheme(&connection).await);
    }
    Ok(())
}

pub(super) fn subscribe() {
    let _ = std::thread::Builder::new()
        .name("sk-power-profiles".into())
        .spawn(|| {
            let _ = futures_lite::future::block_on(monitor());
            set_scheme(Scheme::Balanced);
        });
}

fn stat(text: &str) -> Option<CpuTimes> {
    let mut fields = text.lines().next()?.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let busy = fields
        .by_ref()
        .take(3)
        .try_fold(0u64, |sum, n| Some(sum + n.parse::<u64>().ok()?))?;
    let idle = fields.next()?.parse::<u64>().ok()?;
    let iowait = fields
        .next()
        .map(str::parse::<u64>)
        .transpose()
        .ok()?
        .unwrap_or(0);
    let irq = fields
        .take(3)
        .try_fold(0u64, |sum, n| Some(sum + n.parse::<u64>().ok()?))?;
    Some(CpuTimes {
        busy: busy + irq,
        total: busy + irq + idle + iowait,
    })
}

pub(super) fn cpu_times() -> Option<CpuTimes> {
    let mut line = String::new();
    BufReader::new(fs::File::open("/proc/stat").ok()?)
        .read_line(&mut line)
        .ok()?;
    stat(&line)
}

fn temperature(path: &Path) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn zone_pressure(path: &Path) -> Option<Pressure> {
    let ty = fs::read_to_string(path.join("type")).ok()?;
    let ty = ty.trim().to_ascii_lowercase();
    if !ty.contains("cpu") && ty != "x86_pkg_temp" && ty != "acpitz" {
        return None;
    }
    let current = temperature(&path.join("temp"))?;
    let mut pressure = Pressure::Nominal;
    for id in 0.. {
        let Ok(ty) = fs::read_to_string(path.join(format!("trip_point_{id}_type"))) else {
            break;
        };
        let Some(trip) = temperature(&path.join(format!("trip_point_{id}_temp"))) else {
            continue;
        };
        if trip <= 0 || current < trip {
            continue;
        }
        pressure = pressure.max(match ty.trim() {
            "critical" => Pressure::Critical,
            "hot" | "passive" => Pressure::Serious,
            "active" => Pressure::Fair,
            _ => Pressure::Nominal,
        });
    }
    Some(pressure)
}

pub(super) fn thermal_pressure() -> Pressure {
    fs::read_dir("/sys/class/thermal")
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("thermal_zone")
        })
        .filter_map(|entry| zone_pressure(&entry.path()))
        .max()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_counters_exclude_guest_time_already_counted_in_user_time() {
        let times = stat("cpu 1 2 3 4 5 6 7 8 9 10\ncpu0 1 2 3 4 5 6 7 8 9 10").unwrap();
        assert_eq!(times.busy, 27);
        assert_eq!(times.total, 36);
        assert!(stat("invalid").is_none());
    }

    #[test]
    fn daemon_profiles_fall_back_to_balanced() {
        assert_eq!(profile("power-saver"), Scheme::Saving);
        assert_eq!(profile("performance"), Scheme::Performance);
        assert_eq!(profile("unavailable"), Scheme::Balanced);
    }
}
