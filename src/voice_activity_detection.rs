/// 过零率。说话时过零率会很低，通常低于 0.1。未说话时，背景噪声通常都很杂乱无章，通常会大于 0.3
pub fn zero_crossing_rate<'a>(samples: impl Iterator<Item=&'a f32>) -> f32 {
    let mut prev = 0f32;
    let mut zero_crossing_count = 0usize;
    let mut total_count = 0usize;
    for (i, sample) in samples.enumerate() {
        if i > 0 {
            if prev * sample <= 0.0 {
                zero_crossing_count += 1;
            }
            total_count += 1;
        }
        prev = *sample;
    }
    zero_crossing_count as f32 / total_count as f32
}

/// 短时能量。这个是绝对值，说话时短时能量会突增，需要配合之前一段时间的能量来决定阈值。
pub fn short_time_energy<'a>(samples: impl Iterator<Item=&'a f32>) -> f32 {
    let mut sum = 0f32;
    let mut count = 0usize;
    for sample in samples {
        sum += sample * sample;
        count += 1;
    }
    sum / count as f32
}

/// 声音活动检测。双阈值。
pub fn voice_activity_detection(is_prev_frame_active: bool, zcr: f32, ste: f32, zcr_threshold_low: f32, zcr_threshold_high: f32, ste_min: f32, ste_max: f32, ste_threshold: f32) -> bool {
    if is_prev_frame_active {
        zcr < zcr_threshold_high || ste > ste_min + (ste_max - ste_min) * ste_threshold
    } else {
        zcr < zcr_threshold_low && ste > ste_min + (ste_max - ste_min) * ste_threshold
    }
}

pub struct VoiceActivityDetector {
    zcr_threshold_low: f32,
    zcr_threshold_high: f32,
    ste_min: f32,
    ste_max: f32,
    ste_threshold: f32,
    last_active_ste: f32,
    is_prev_frame_active: bool,
}

impl Default for VoiceActivityDetector {
    fn default() -> Self {
        Self::new(0.1, 0.3, 0.1)
    }
}

impl VoiceActivityDetector {
    pub fn new(zcr_threshold_low: f32, zcr_threshold_high: f32, ste_threshold: f32) -> Self {
        Self {
            zcr_threshold_low,
            zcr_threshold_high,
            ste_min: f32::MAX,
            ste_max: 0.0,
            ste_threshold,
            last_active_ste: 0.0,
            is_prev_frame_active: false,
        }
    }

    pub fn detect<'a>(&mut self, samples: impl Iterator<Item=&'a f32> + Clone) -> bool {
        let zcr = zero_crossing_rate(samples.clone());
        let ste = short_time_energy(samples);
        if ste > self.ste_max {
            self.ste_max = ste;
        }
        if ste < self.ste_min {
            self.ste_min = ste;
            if self.last_active_ste < self.ste_min {
                self.last_active_ste = self.ste_min + (self.ste_max - self.ste_min) * self.ste_threshold;
            }
        }
        let active = voice_activity_detection(self.is_prev_frame_active, zcr, ste, self.zcr_threshold_low, self.zcr_threshold_high, self.ste_min, self.ste_max, self.ste_threshold);
        self.is_prev_frame_active = active;
        if active {
            self.last_active_ste = ste;
        } else {
            // 如果出现极端数值，让 ste_min 和 ste_max 缓慢地回归到正常值
            self.ste_min = self.ste_min + (self.last_active_ste - self.ste_min) * 0.001;
            if self.ste_min + (self.ste_max - self.ste_min) * self.ste_threshold > self.last_active_ste {
                self.ste_max = self.ste_max - (self.ste_max - self.last_active_ste) * 0.01;
            }
        }
        active
    }
}
