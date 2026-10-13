// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書の時計。boa の `Date` はマルチスレッドではタイムゾーンが取れず UTC になるので、
//! [`Jst`] で +9 時間に固定する。`subDate` が一万ミリ秒を超えれば録画と見なされる。

use std::cell::Cell;
use std::rc::Rc;

use boa_engine::context::time::JsInstant;
use boa_engine::context::{Clock, HostHooks};

pub const JST_OFFSET_SECONDS: i32 = 9 * 3600;

#[derive(Debug, Default, Clone, Copy)]
pub struct Jst;

impl HostHooks for Jst {
    fn local_timezone_offset_seconds(&self, _unix_time_seconds: i64) -> i32 {
        JST_OFFSET_SECONDS
    }
}

#[derive(Debug)]
pub struct BmlClock {
    base_ms: f64,
    offset_ms: Cell<i64>,
}

impl Default for BmlClock {
    fn default() -> Self {
        Self::new()
    }
}

impl BmlClock {
    pub fn new() -> Self {
        Self {
            base_ms: zwsdk::plug::host::monotonic_ms(),
            offset_ms: Cell::new(0),
        }
    }

    pub fn set_offset_ms(&self, offset: i64) {
        self.offset_ms.set(offset);
    }

    pub fn machine_millis() -> i64 {
        zwsdk::plug::host::unix_ms() as i64
    }
}

impl Clock for BmlClock {
    fn now(&self) -> JsInstant {
        let elapsed_ms = (zwsdk::plug::host::monotonic_ms() - self.base_ms).max(0.0);
        let secs = (elapsed_ms / 1000.0) as u64;
        let nanos = ((elapsed_ms % 1000.0) * 1_000_000.0) as u32;
        JsInstant::new(secs, nanos)
    }

    fn system_time_millis(&self) -> i64 {
        Self::machine_millis() + self.offset_ms.get()
    }
}

pub fn context(clock: Rc<BmlClock>) -> boa_engine::Context {
    let mut context = boa_engine::Context::builder()
        .host_hooks(Rc::new(Jst))
        .clock(clock)
        .build()
        .expect("a fresh context");
    limit(&mut context);
    context
}

pub const LOOP_MOST: u64 = 10_000_000;

pub const RECURSION_MOST: usize = 512;

/// 放送の文書が `while(1){}` や深い再帰でスレッドを取ったままにしないよう、boa に上限を置く。
pub fn limit(context: &mut boa_engine::Context) {
    let limits = context.runtime_limits_mut();
    limits.set_loop_iteration_limit(LOOP_MOST);
    limits.set_recursion_limit(RECURSION_MOST);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

pub fn civil(millis: f64, offset_seconds: i32) -> Civil {
    let seconds = (millis / 1000.0).floor() as i64 + i64::from(offset_seconds);
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    // 日数から暦へ（Howard Hinnant の civil_from_days）。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    Civil {
        year,
        month,
        day,
        hour: (of_day / 3600) as u32,
        minute: (of_day % 3600 / 60) as u32,
        second: (of_day % 60) as u32,
    }
}

pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl Civil {
    pub fn bml(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_are_right() {
        let c = civil(1_789_171_200_000.0, 0);
        assert_eq!(c.bml(), "2026-09-12T00:00:00");
        assert_eq!(
            civil(1_789_171_200_000.0, JST_OFFSET_SECONDS).bml(),
            "2026-09-12T09:00:00"
        );
        assert_eq!(civil(0.0, -1).bml(), "1969-12-31T23:59:59");
        assert_eq!(civil(951_782_400_000.0, 0).bml(), "2000-02-29T00:00:00");
    }

    #[test]
    fn days_from_civil_is_the_inverse_of_civil() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 9, 12), 20_708);
        for millis in [0.0, 951_782_400_000.0, 1_789_171_200_000.0] {
            let c = civil(millis, 0);
            assert_eq!(
                days_from_civil(c.year, c.month, c.day) * 86_400_000,
                millis as i64
            );
        }
    }

    #[test]
    fn the_clock_can_be_shifted() {
        let clock = BmlClock::new();
        let before = clock.system_time_millis();
        clock.set_offset_ms(-3_600_000);
        assert!(clock.system_time_millis() < before - 3_500_000);
    }
}
