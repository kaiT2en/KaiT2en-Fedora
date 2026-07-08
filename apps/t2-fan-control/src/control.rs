use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use crate::{
    config::{curve_for_preset, AppConfig, CurvePoint},
    error::Result,
    sysfs::{FanEndpoint, TemperatureSnapshot, TemperatureSource},
};

pub struct Controller {
    samples: VecDeque<u8>,
    last_applied_percent: Option<u8>,
    last_tick: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct ControlSnapshot {
    pub temperatures: TemperatureSnapshot,
    pub effective_temp_c: Option<u8>,
    pub target_percent: Option<u8>,
    pub target_rpm_per_fan: Vec<u32>,
}

impl Controller {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(50),
            last_applied_percent: None,
            last_tick: Instant::now() - Duration::from_secs(5),
        }
    }

    pub fn tick(
        &mut self,
        config: &AppConfig,
        fans: &mut [FanEndpoint],
        temperatures: &mut [TemperatureSource],
    ) -> Result<ControlSnapshot> {
        let snapshot = TemperatureSnapshot::read_from(temperatures);
        let effective_temp = snapshot.effective_temp_c();

        if let Some(temp) = effective_temp {
            self.samples.push_back(temp);
            if self.samples.len() > 50 {
                self.samples.pop_front();
            }
        }

        let smoothed_temp = self.smoothed_temp();
        let curve = curve_for_preset(config);
        let target_percent = smoothed_temp.map(|temp| interpolate_percent(&curve, temp));

        let mut target_rpm_per_fan = Vec::with_capacity(fans.len());
        if config.automatic_control_enabled {
            let should_apply = should_apply_target(self.last_applied_percent, target_percent);

            for fan in fans {
                let rpm = target_percent
                    .map(|percent| fan.percent_to_rpm(percent))
                    .unwrap_or(fan.min_speed);

                if should_apply {
                    fan.set_target_speed(rpm)?;
                    fan.current_speed = Some(rpm);
                    fan.app_controlled = Some(true);
                }

                target_rpm_per_fan.push(rpm);
            }

            if should_apply {
                self.last_applied_percent = target_percent;
            }
        }

        self.last_tick = Instant::now();

        Ok(ControlSnapshot {
            temperatures: snapshot,
            effective_temp_c: smoothed_temp,
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
        Ok(())
    }

    pub fn should_tick(&self) -> bool {
        self.last_tick.elapsed() >= Duration::from_millis(800)
    }

    fn smoothed_temp(&self) -> Option<u8> {
        if self.samples.is_empty() {
            return None;
        }

        let sum: u16 = self.samples.iter().map(|value| *value as u16).sum();
        Some((sum / self.samples.len() as u16) as u8)
    }
}

fn should_apply_target(last_applied_percent: Option<u8>, next_target_percent: Option<u8>) -> bool {
    match (last_applied_percent, next_target_percent) {
        (None, Some(_)) | (Some(_), None) => true,
        (None, None) => false,
        (Some(previous), Some(next)) => previous.abs_diff(next) >= 3,
    }
}

fn interpolate_percent(curve: &[CurvePoint], temp_c: u8) -> u8 {
    if curve.is_empty() {
        return 0;
    }
    if temp_c <= curve[0].temp_c {
        return curve[0].speed_percent;
    }
    for window in curve.windows(2) {
        let left = &window[0];
        let right = &window[1];
        if temp_c <= right.temp_c {
            let temp_span = (right.temp_c - left.temp_c) as f32;
            if temp_span <= f32::EPSILON {
                return right.speed_percent;
            }
            let progress = (temp_c - left.temp_c) as f32 / temp_span;
            let speed_span = right.speed_percent as f32 - left.speed_percent as f32;
            return (left.speed_percent as f32 + progress * speed_span).round() as u8;
        }
    }
    curve.last().map(|point| point.speed_percent).unwrap_or(100)
}
