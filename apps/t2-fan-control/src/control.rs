// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
use std::time::{Duration, Instant};

use crate::{
    config::{curve, curve_percent, AppConfig, CurvePoint},
    error::Result,
    sysfs::{FanEndpoint, TemperatureSnapshot, TemperatureSource},
};

const CONTROL_INTERVAL: Duration = Duration::from_secs(2);
const SYSTEM_LIMIT_HYSTERESIS_C: u8 = 2;
const SYSTEM_STABLE_RELEASE_TIME: Duration = Duration::from_secs(20);
const RAMP_UP_STEP_INTERVAL: Duration = Duration::from_millis(100);
const RAMP_DOWN_STEP_INTERVAL: Duration = Duration::from_millis(500);
const RAMP_START_THRESHOLD_PERCENT: u8 = 3;

pub struct Controller {
    last_applied_percent: Option<u8>,
    goal_percent: Option<u8>,
    ramping: bool,
    next_ramp_step: Instant,
    last_tick: Instant,
    heat_soak_cooling: bool,
    any_sensor_cooling: bool,
    system_cooling_started_at: Option<Instant>,
    system_below_target_since: Option<Instant>,
}

#[derive(Clone, Debug, Default)]
pub struct ControlSnapshot {
    pub temperatures: TemperatureSnapshot,
    pub effective_temp_c: Option<u8>,
    pub system_temp_c: Option<u8>,
    pub system_sensor_count: usize,
    pub heat_soak_cooling: bool,
    pub any_sensor_cooling: bool,
    pub target_percent: Option<u8>,
    pub target_rpm_per_fan: Vec<u32>,
}

impl Controller {
    pub fn new() -> Self {
        Self {
            last_applied_percent: None,
            goal_percent: None,
            ramping: false,
            next_ramp_step: Instant::now(),
            last_tick: Instant::now() - CONTROL_INTERVAL,
            heat_soak_cooling: false,
            any_sensor_cooling: false,
            system_cooling_started_at: None,
            system_below_target_since: None,
        }
    }

    pub fn tick(
        &mut self,
        config: &AppConfig,
        fans: &mut [FanEndpoint],
        temperatures: &mut [TemperatureSource],
        refresh_all_sensors: bool,
    ) -> Result<ControlSnapshot> {
        let mut snapshot = TemperatureSnapshot::read_for_control(
            temperatures,
            config.curve_sensor_key.as_deref(),
            refresh_all_sensors,
        );
        snapshot.include_curve_sensor(temperatures, config.curve_sensor_key.as_deref());
        let effective_temp = snapshot.effective_temp_c();

        let curve = curve(config);
        let curve_target = effective_temp.map(|temp| interpolate_percent(&curve, temp));
        self.update_system_cooling(config, snapshot.system_temp_c);
        self.any_sensor_cooling = if config.any_sensor_enabled {
            next_threshold_cooling(
                self.any_sensor_cooling,
                snapshot.overall_hottest_temp_c,
                config.any_sensor_temp_c,
                config.any_sensor_temp_c.saturating_sub(SYSTEM_LIMIT_HYSTERESIS_C),
            )
        } else {
            false
        };
        let above_curve = effective_temp
            .zip(curve.last())
            .is_some_and(|(temp, end)| temp > end.temp_c);
        let forced = self.heat_soak_cooling || self.any_sensor_cooling || above_curve;
        let mut target_percent = if forced { Some(100) } else { curve_target };

        let mut target_rpm_per_fan = Vec::new();
        if config.automatic_control_enabled {
            self.goal_percent = target_percent;
            let small_change = self.last_applied_percent.zip(target_percent)
                .is_some_and(|(applied, goal)| applied.abs_diff(goal) < RAMP_START_THRESHOLD_PERCENT);
            if forced || self.last_applied_percent.is_none() || target_percent.is_none() {
                self.ramping = false;
                if target_percent != self.last_applied_percent {
                    self.apply(fans, target_percent)?;
                }
            } else if !self.ramping && !small_change {
                self.ramping = true;
                self.next_ramp_step = Instant::now();
            }
            target_percent = self.last_applied_percent;
            target_rpm_per_fan = self.target_rpm_per_fan(fans);
        }

        self.last_tick = Instant::now();

        let system_temp_c = snapshot.system_temp_c;
        let system_sensor_count = snapshot.system_sensor_count;
        Ok(ControlSnapshot {
            temperatures: snapshot,
            effective_temp_c: effective_temp,
            system_temp_c,
            system_sensor_count,
            heat_soak_cooling: self.heat_soak_cooling,
            any_sensor_cooling: self.any_sensor_cooling,
            target_percent,
            target_rpm_per_fan,
        })
    }

    pub fn release_to_system(&mut self, fans: &mut [FanEndpoint]) -> Result<()> {
        for fan in fans {
            fan.release_to_auto()?;
            fan.app_controlled = Some(false);
        }
        self.last_applied_percent = None;
        self.goal_percent = None;
        self.ramping = false;
        self.heat_soak_cooling = false;
        self.any_sensor_cooling = false;
        self.system_cooling_started_at = None;
        self.system_below_target_since = None;
        Ok(())
    }

    pub fn should_tick(&self) -> bool {
        self.last_tick.elapsed() >= CONTROL_INTERVAL
    }

    /// Takes one 1% step toward the goal when it is due. Returns whether the fans changed.
    pub fn ramp(&mut self, fans: &mut [FanEndpoint]) -> Result<bool> {
        if !self.ramping || Instant::now() < self.next_ramp_step {
            return Ok(false);
        }
        let (Some(applied), Some(goal)) = (self.last_applied_percent, self.goal_percent) else {
            self.ramping = false;
            return Ok(false);
        };
        let next = ramp_step(applied, goal);
        self.apply(fans, Some(next))?;
        self.ramping = next != goal;
        self.next_ramp_step = Instant::now()
            + if goal > applied { RAMP_UP_STEP_INTERVAL } else { RAMP_DOWN_STEP_INTERVAL };
        Ok(true)
    }

    /// Time until the next control tick or ramp step.
    pub fn next_wakeup(&self) -> Duration {
        let control = CONTROL_INTERVAL.saturating_sub(self.last_tick.elapsed());
        if self.ramping {
            control.min(self.next_ramp_step.saturating_duration_since(Instant::now()))
        } else {
            control
        }
    }

    pub fn applied_percent(&self) -> Option<u8> {
        self.last_applied_percent
    }

    pub fn target_rpm_per_fan(&self, fans: &[FanEndpoint]) -> Vec<u32> {
        fans.iter()
            .map(|fan| self.last_applied_percent.map(|percent| fan.percent_to_rpm(percent)).unwrap_or(fan.min_speed))
            .collect()
    }

    fn apply(&mut self, fans: &mut [FanEndpoint], percent: Option<u8>) -> Result<()> {
        for fan in fans {
            let rpm = percent.map(|percent| fan.percent_to_rpm(percent)).unwrap_or(fan.min_speed);
            fan.set_target_speed(rpm)?;
            fan.current_speed = Some(rpm);
            fan.app_controlled = Some(true);
        }
        self.last_applied_percent = percent;
        Ok(())
    }

    fn update_system_cooling(&mut self, config: &AppConfig, system_temp_c: Option<u8>) {
        let now = Instant::now();
        let engage = config.soak_temp_c.saturating_add(SYSTEM_LIMIT_HYSTERESIS_C);
        let release = config.soak_temp_c.saturating_sub(SYSTEM_LIMIT_HYSTERESIS_C);

        if !self.heat_soak_cooling {
            if system_temp_c.is_some_and(|temp| temp >= engage) {
                self.heat_soak_cooling = true;
                self.system_cooling_started_at = Some(now);
                self.system_below_target_since = None;
            }
            return;
        }

        if system_temp_c.is_some_and(|temp| temp <= release) {
            self.system_below_target_since.get_or_insert(now);
        } else {
            self.system_below_target_since = None;
        }

        let minimum_elapsed = self.system_cooling_started_at
            .is_some_and(|started| now.duration_since(started) >= Duration::from_secs(config.system_cooling_time_s as u64));
        let stable_elapsed = self.system_below_target_since
            .is_some_and(|started| now.duration_since(started) >= SYSTEM_STABLE_RELEASE_TIME);
        if minimum_elapsed && stable_elapsed {
            self.heat_soak_cooling = false;
            self.system_cooling_started_at = None;
            self.system_below_target_since = None;
        }
    }

}

fn next_threshold_cooling(active: bool, temp_c: Option<u8>, engage_temp_c: u8, release_temp_c: u8) -> bool {
    match temp_c {
        Some(temp) if temp >= engage_temp_c => true,
        Some(temp) if temp <= release_temp_c => false,
        _ => active,
    }
}

fn ramp_step(applied: u8, goal: u8) -> u8 {
    if goal > applied { applied + 1 } else { applied.saturating_sub(1).max(goal) }
}

fn interpolate_percent(curve: &[CurvePoint], temp_c: u8) -> u8 {
    curve_percent(curve, temp_c as f64).round().clamp(0.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::{interpolate_percent, next_threshold_cooling, ramp_step};
    use crate::config::CurvePoint;

    #[test]
    fn system_target_has_plus_minus_two_degree_hysteresis() {
        assert!(next_threshold_cooling(false, Some(47), 47, 43));
        assert!(next_threshold_cooling(true, Some(45), 47, 43));
        assert!(next_threshold_cooling(true, Some(44), 47, 43));
        assert!(!next_threshold_cooling(true, Some(43), 47, 43));
    }

    #[test]
    fn temperature_above_last_curve_point_forces_full_speed() {
        let curve = vec![
            CurvePoint { temp_c: 0, speed_percent: 0 },
            CurvePoint { temp_c: 70, speed_percent: 30 },
            CurvePoint { temp_c: 90, speed_percent: 50 },
        ];
        assert_eq!(interpolate_percent(&curve, 90), 50);
        assert_eq!(interpolate_percent(&curve, 91), 100);
        assert_eq!(interpolate_percent(&curve, 100), 100);
    }

    #[test]
    fn ramp_moves_one_percent_toward_goal() {
        assert_eq!(ramp_step(20, 40), 21);
        assert_eq!(ramp_step(40, 20), 39);
        assert_eq!(ramp_step(20, 20), 20);
    }
}
