//! Memory and swap from `/proc/meminfo`, read as gopsutil reads it and combined as the Go agent's
//! `calculateHostMemoryUsage` combines it.

#[derive(Debug, Default, PartialEq)]
pub struct Memory {
    pub total: u64,
    pub used: u64,
    pub buff_cache: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

#[derive(Default)]
struct MemInfo {
    total: u64,
    free: u64,
    available: Option<u64>,
    buffers: u64,
    cached: u64,
    reclaimable: u64,
    shared: u64,
    swap_total: u64,
    swap_free: u64,
}

/// `None` when a field the agent uses does not parse, as gopsutil then fails the whole read.
/// `htop` is `MEM_CALC=htop`: used memory without buffers and cache.
pub fn parse(meminfo: &str, htop: bool) -> Option<Memory> {
    let mut m = MemInfo::default();
    for line in meminfo.lines() {
        let mut parts = line.split(':');
        let (Some(key), Some(value), None) = (parts.next(), parts.next(), parts.next()) else { continue };
        let slot = match key.trim() {
            "MemTotal" => &mut m.total,
            "MemFree" => &mut m.free,
            "MemAvailable" => m.available.insert(0),
            "Buffers" => &mut m.buffers,
            "Cached" => &mut m.cached,
            "SReclaimable" => &mut m.reclaimable,
            "Shmem" => &mut m.shared,
            "SwapTotal" => &mut m.swap_total,
            "SwapFree" => &mut m.swap_free,
            _ => continue,
        };
        *slot = value.trim().replace(" kB", "").parse::<u64>().ok()?.wrapping_mul(1024);
    }
    let cached = m.cached + m.reclaimable;
    let available = m.available.unwrap_or(cached + m.free);
    // meminfo can change between its lines, so every difference saturates.
    let mut used = m.total.wrapping_sub(available);
    if used > m.total {
        used = m.total.saturating_sub(available);
    }
    let mut buff_cache = cached.min(m.total);
    buff_cache += m.buffers.min(m.total - buff_cache);
    buff_cache = buff_cache.saturating_sub(m.shared.min(m.total));
    if cached == 0 && m.buffers == 0 {
        buff_cache = m.total.saturating_sub(m.free).saturating_sub(used);
    }
    if htop {
        used = m.total.saturating_sub(m.free).saturating_sub(buff_cache);
    }
    Some(Memory { total: m.total, used, buff_cache, swap_total: m.swap_total, swap_used: m.swap_total.saturating_sub(m.swap_free) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:        2000 kB\nMemFree:          500 kB\nMemAvailable:    1200 kB\nBuffers:           50 kB\n\
                           Cached:           600 kB\nSwapCached:         0 kB\nShmem:            100 kB\nSReclaimable:      40 kB\n\
                           SwapTotal:        800 kB\nSwapFree:         600 kB\n";

    #[test]
    fn combines_meminfo_as_the_go_agent() {
        let kb = |n: u64| n * 1024;
        let memory = parse(MEMINFO, false).unwrap();
        assert_eq!(
            memory,
            Memory { total: kb(2000), used: kb(800), buff_cache: kb(590), swap_total: kb(800), swap_used: kb(200) }
        );
        assert_eq!(parse(MEMINFO, true).unwrap().used, kb(2000 - 500 - 590));
        assert_eq!(parse("MemTotal: lots kB\n", false), None);
    }
}
