mod recorder;

use crate::recorder::Recorder;
use bing_stt::speech_recognition::build_wave_header_from_wave_format;
use bing_stt::{Session, VoiceActivityDetector};
use std::slice;
use std::thread::sleep;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let mut vad = VoiceActivityDetector::default();
    let mut recorder = Recorder::new()?;
    let device_name = recorder.device_name()?;
    println!("Device name: {}", device_name);

    let wave_format = recorder.wave_format();
    let channels = wave_format.as_ref().nChannels as usize;
    let wave_header = build_wave_header_from_wave_format(&wave_format.as_bytes());

    let mut speech_recognition_session: Option<Session> = None;
    let mut stop_time = Instant::now();
    loop {
        if let Some(session) = &mut speech_recognition_session {
            if let Some((text, finished)) = session.try_recv_message()? {
                if finished {
                    println!("------> {}", text);
                    stop_time = Instant::now();
                    speech_recognition_session = None;
                } else {
                    println!("{} ...", text);
                }
            }
        }

        recorder.capture(|captured_buffer| {
            let f32_buffer = unsafe { slice::from_raw_parts(captured_buffer.as_ptr() as *const f32, captured_buffer.len() / 4) };
            let now = Instant::now();
            let is_active = vad.detect(f32_buffer.iter().step_by(channels));
            if is_active || now < stop_time {
                stop_time = now + Duration::from_secs(3);
                if speech_recognition_session.is_none() {
                    match Session::new("zh-CN") {
                        Ok(mut session) => {
                            println!("======> Session created");
                            match session.write(&wave_header) {
                                Ok(_) => {
                                    speech_recognition_session = Some(session);
                                }
                                Err(e) => {
                                    eprintln!("Failed to write wave header: {}", e);
                                    speech_recognition_session = None;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to create session: {}", e);
                        }
                    }
                }
                if let Some(session) = &mut speech_recognition_session {
                    match session.write(captured_buffer) {
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("Failed to write captured buffer: {}", e);
                            speech_recognition_session = None;
                        }
                    }
                }
            }
        })?;
        sleep(Duration::from_millis(10));
    }
}
