use super::CpuBreakdown;
use super::read_cpu_breakdown_raw;

pub(super) fn cpu_percent(value: f32) -> u8 { rounded_percent(f64::from(value)) }

pub(super) fn normalize_cpu_label(name: &str, index: usize) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        format!("CPU {}", index + 1)
    } else {
        trimmed.to_string()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CpuBreakdownRaw {
    pub(super) system: u64,
    pub(super) user:   u64,
    pub(super) idle:   u64,
}

pub(super) fn cpu_breakdown(previous: &mut CpuBreakdownRaw) -> CpuBreakdown {
    let current = read_cpu_breakdown_raw();
    let delta_system = current.system.saturating_sub(previous.system);
    let delta_user = current.user.saturating_sub(previous.user);
    let delta_idle = current.idle.saturating_sub(previous.idle);
    let delta_total = delta_system
        .saturating_add(delta_user)
        .saturating_add(delta_idle);
    *previous = current;

    if delta_total == 0 {
        return CpuBreakdown::default();
    }

    CpuBreakdown {
        system: percent_from_parts(delta_system, delta_total),
        user:   percent_from_parts(delta_user, delta_total),
        idle:   percent_from_parts(delta_idle, delta_total),
    }
}

pub(super) fn percent_from_parts(value: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let rounded = value.saturating_mul(100).saturating_add(total / 2) / total;
    bounded_percent_u8(rounded)
}

pub(super) fn rounded_percent(value: f64) -> u8 {
    let clamped = value.clamp(0.0, 100.0);
    let mut percent = 0u8;
    while percent < 100 && f64::from(percent) + 0.5 <= clamped {
        percent += 1;
    }
    percent
}

pub(super) fn bounded_percent_u8(value: u64) -> u8 { u8::try_from(value.min(100)).unwrap_or(100) }
