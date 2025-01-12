use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::net::TcpStream;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use websocket::header::Headers;
use websocket::native_tls::{TlsConnector, TlsStream};
use websocket::sync::Client;
use websocket::{ClientBuilder, Message, OwnedMessage, WebSocketError};

pub fn get_timestamp() -> String {
    OffsetDateTime::now_utc().format(&Rfc3339).unwrap()
}

pub fn random_request_id() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode_upper(&buf[..])
}

pub fn get_request_url(uqurequestid: &str, x_connection_id: &str) -> String {
    format!("wss://sr.bing.com/opaluqu/speech/recognition/interactive/cognitiveservices/v1?clientbuild=bingDesktop&referer=https%3A%2F%2Fwww.bing.com%2F&form=QBLH&uqurequestid={}&language=xx-yy&format=simple&Ocp-Apim-Subscription-Key=key&X-ConnectionId={}", uqurequestid, x_connection_id)
}

/// # Arguments
/// * `format_tag` - 1: PCM, 3: IEEE_FLOAT PCM
/// * `channels` - 1: mono, 2: stereo
/// * `samples_per_sec` - 采样率
/// * `bits_per_sample` - 采样位数
pub fn build_wave_header(
    format_tag: u16,
    channels: u16,
    samples_per_sec: u32,
    bits_per_sample: u16,
) -> Vec<u8> {
    let block_align = channels * bits_per_sample / 8;
    let avg_bytes_per_sec = samples_per_sec * block_align as u32;
    let mut buf = Vec::<u8>::with_capacity(40);
    buf.write_all(b"RIFF").unwrap();
    buf.write_all(&[0x00, 0x00, 0x00, 0x00]).unwrap();
    buf.write_all(b"WAVE").unwrap();
    buf.write_all(b"fmt ").unwrap();
    buf.write_all(&[0x10, 0x00, 0x00, 0x00]).unwrap(); // size of WAVEFORMATEX
    buf.push((format_tag & 0xff) as u8);
    buf.push(((format_tag >> 8) & 0xff) as u8);
    buf.push((channels & 0xff) as u8);
    buf.push(((channels >> 8) & 0xff) as u8);
    buf.push((samples_per_sec & 0xff) as u8);
    buf.push(((samples_per_sec >> 8) & 0xff) as u8);
    buf.push(((samples_per_sec >> 16) & 0xff) as u8);
    buf.push(((samples_per_sec >> 24) & 0xff) as u8);
    buf.push((avg_bytes_per_sec & 0xff) as u8);
    buf.push(((avg_bytes_per_sec >> 8) & 0xff) as u8);
    buf.push(((avg_bytes_per_sec >> 16) & 0xff) as u8);
    buf.push(((avg_bytes_per_sec >> 24) & 0xff) as u8);
    buf.push((block_align & 0xff) as u8);
    buf.push(((block_align >> 8) & 0xff) as u8);
    buf.push((bits_per_sample & 0xff) as u8);
    buf.push(((bits_per_sample >> 8) & 0xff) as u8);
    buf.write_all(b"data").unwrap();
    buf
}

pub fn build_wave_header_from_wave_format(wave_format_bytes: impl AsRef<[u8]>) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.write_all(b"RIFF").unwrap();
    buf.write_all(&[0x00, 0x00, 0x00, 0x00]).unwrap();
    buf.write_all(b"WAVE").unwrap();
    buf.write_all(b"fmt ").unwrap();
    let wave_format_bytes = wave_format_bytes.as_ref();
    let wave_format_bytes_len = wave_format_bytes.len();
    buf.push((wave_format_bytes_len & 0xff) as u8);
    buf.push(((wave_format_bytes_len >> 8) & 0xff) as u8);
    buf.push(((wave_format_bytes_len >> 16) & 0xff) as u8);
    buf.push(((wave_format_bytes_len >> 24) & 0xff) as u8);
    buf.write_all(wave_format_bytes).unwrap();
    buf.write_all(b"data").unwrap();
    buf
}

pub fn split_header_body(s: impl AsRef<str>) -> (String, String) {
    let mut iter = s.as_ref().splitn(2, "\r\n\r\n");
    let header = iter.next().unwrap();
    let body = iter.next().unwrap_or("");
    (header.to_owned(), body.to_owned())
}

pub fn parse_headers(s: impl AsRef<str>) -> Vec<(String, String)> {
    s.as_ref()
        .split("\r\n")
        .filter_map(|s| {
            if s.len() > 0 {
                let mut iter = s.splitn(2, ":");
                let k = iter.next().unwrap_or("").trim().to_owned();
                let v = iter.next().unwrap_or("").trim().to_owned();
                Some((k, v))
            } else {
                None
            }
        })
        .collect()
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechHypothesis {
    #[serde(rename = "Text")]
    pub text: String,
    #[serde(rename = "Offset")]
    pub offset: i64,
    #[serde(rename = "Duration")]
    pub duration: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechPhrase {
    #[serde(rename = "RecognitionStatus")]
    pub recognition_status: String,
    #[serde(rename = "Offset")]
    pub offset: i64,
    #[serde(rename = "Duration")]
    pub duration: i64,
    #[serde(rename = "DisplayText", default)]
    pub display_text: String,
}

pub const FLUSH_SIZE: usize = 3300;

pub struct Session {
    client: Client<TlsStream<TcpStream>>,
    request_id: String,
    buffer: Vec<u8>,
}

impl Session {
    /// # Arguments
    /// * `default_language` - "zh-CN", "en-US"
    /// # Returns
    /// * Err, when websocket error occurs
    pub fn new(default_language: &str) -> anyhow::Result<Self> {
        let uqurequestid = random_request_id();
        let x_connection_id = random_request_id();
        let request_id = random_request_id();
        let request_url = get_request_url(&uqurequestid, &x_connection_id);
        let mut headers = Headers::new();
        headers.append_raw("Accept-Language", default_language.as_bytes().to_vec());
        headers.append_raw("User-Agent", b"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0".to_vec());
        let mut client = ClientBuilder::new(&request_url)?
            .custom_headers(&headers)
            .connect_secure(Some(TlsConnector::new().unwrap()))?;
        let _ = client.set_nonblocking(true);
        client.send_message(&Message::text(format!("Path: speech.config\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\nContent-Type: application/json\r\n\r\n{}", request_id, get_timestamp(), r#"{"context":{"system":{"name":"SpeechSDK","version":"1.15.0-alpha.0.1","build":"JavaScript","lang":"JavaScript"},"os":{"platform":"Browser/Win32","name":"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0","version":"5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0"},"audio":{"source":{"bitspersample":16,"channelcount":1,"connectivity":"Unknown","manufacturer":"Speech SDK","model":"Default - Microphone","samplerate":16000,"type":"Microphones"}}},"recognition":"interactive"}"#)))?;
        client.send_message(&Message::text(format!("Path: speech.context\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\nContent-Type: application/json\r\n\r\n{}", request_id, get_timestamp(), "{}")))?;
        let mut buffer = Vec::with_capacity(FLUSH_SIZE);
        let header = format!("Path: audio\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\nContent-Type: audio/x-wav\r\n", &request_id, get_timestamp());
        let header_bytes_len = header.len();
        buffer.push(((header_bytes_len >> 8) & 0xff) as u8);
        buffer.push((header_bytes_len & 0xff) as u8);
        buffer.write_all(header.as_bytes())?;
        Ok(Self {
            client,
            request_id,
            buffer,
        })
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) -> anyhow::Result<()> {
        self.buffer.extend_from_slice(data.as_ref());
        if self.buffer.len() >= FLUSH_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        if self.buffer.len() > 0 {
            self.client.send_message(&Message::binary(self.buffer.clone()))?;
            self.buffer.clear();
            let header = format!("Path: audio\r\nX-RequestId: {}\r\nX-Timestamp: {}\r\n", &self.request_id, get_timestamp());
            let header_bytes_len = header.len();
            self.buffer.push(((header_bytes_len >> 8) & 0xff) as u8);
            self.buffer.push((header_bytes_len & 0xff) as u8);
            self.buffer.write_all(header.as_bytes())?;
        }
        Ok(())
    }

    /// # Returns
    /// * Err(anyhow::Error), when websocket error occurs
    /// * Ok(None), when no message is available
    /// * Ok(Some((text, is_final))) When is_final is false, it's a partial text. This part of text may change in the final result.
    pub fn try_recv_message(&mut self) -> anyhow::Result<Option<(String, bool)>> {
        match self.client.recv_message() {
            Ok(msg) => {
                if let OwnedMessage::Text(text) = msg {
                    let (header_text, body_text) = split_header_body(text.as_str());
                    let headers = parse_headers(header_text);
                    for (key, value) in headers.iter() {
                        if key == "Path" && value == "speech.hypothesis" {
                            let v = serde_json::from_str::<SpeechHypothesis>(&body_text)?;
                            return Ok(Some((v.text, false)));
                        } else if key == "Path" && value == "speech.phrase" {
                            let v = serde_json::from_str::<SpeechPhrase>(&body_text)?;
                            return Ok(Some((v.display_text, true)));
                        } else if key == "Path" && value == "turn.end" {
                            self.client.shutdown()?;
                            return Ok(None);
                        }
                    }
                }
                Ok(None)
            }
            Err(e) => {
                if let WebSocketError::IoError(e) = &e {
                    if e.kind() == std::io::ErrorKind::WouldBlock {
                        return Ok(None);
                    }
                }
                Err(e.into())
            }
        }
    }
}
