//! Microphone and speaker on `cpal` (R27.5, R28.6), behind the `audio` feature.
//!
//! The harness speaks 16-bit PCM at 24 kHz mono (the Realtime format); devices rarely do,
//! so capture downmixes and resamples to it and playback resamples and upmixes from it.
//! `cpal` streams are not `Send`, so each one lives on its own thread and talks to the
//! async side through channels.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use futures::StreamExt as _;

use super::{AudioFormat, AudioStream, HostError, Microphone, Speaker};
use crate::harness_async_trait;
use crate::runtime::boxed_stream;

/// The rate the harness speaks.
pub const RATE: u32 = 24_000;
const PCM24K: AudioFormat = AudioFormat::Pcm16 {
    sample_rate: RATE,
    channels: 1,
};

/// Little-endian 16-bit PCM as samples in `[-1, 1]`.
#[must_use]
pub fn pcm16_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes(*b)) / 32_768.0)
        .collect()
}

/// Samples in `[-1, 1]` as little-endian 16-bit PCM (clipped).
#[must_use]
pub fn f32_to_pcm16(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| {
            #[allow(clippy::cast_possible_truncation)]
            let v = (s.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
            v.to_le_bytes()
        })
        .collect()
}

/// Linear resampling from `from` to `to` samples per second.
#[must_use]
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let len = ((input.len() as f64) / ratio).floor() as usize;
    (0..len)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let pos = i as f64 * ratio;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let at = pos.floor() as usize;
            #[allow(clippy::cast_possible_truncation)]
            let frac = (pos - pos.floor()) as f32;
            let a = input[at.min(input.len() - 1)];
            let b = input[(at + 1).min(input.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

/// Interleaved frames averaged to mono.
#[must_use]
pub fn downmix(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let n = usize::from(channels.max(1));
    interleaved
        .chunks(n)
        .map(|frame| {
            #[allow(clippy::cast_precision_loss)]
            let len = frame.len() as f32;
            frame.iter().sum::<f32>() / len
        })
        .collect()
}

/// Mono samples repeated across `channels`.
#[must_use]
pub fn upmix(mono: &[f32], channels: u16) -> Vec<f32> {
    let n = usize::from(channels.max(1));
    mono.iter()
        .flat_map(|s| std::iter::repeat_n(*s, n))
        .collect()
}

fn device_error(e: impl std::fmt::Display) -> HostError {
    HostError::Io(format!("audio: {e}"))
}

fn require_pcm24k(format: AudioFormat) -> Result<(), HostError> {
    if format == PCM24K {
        Ok(())
    } else {
        Err(HostError::Denied(format!(
            "audio is 16-bit PCM at 24 kHz mono here, not {format:?}"
        )))
    }
}

/// The default input device.
#[derive(Debug, Default, Clone, Copy)]
pub struct CpalMicrophone;

/// The default output device.
#[derive(Debug, Default, Clone, Copy)]
pub struct CpalSpeaker;

type Chunks = tokio::sync::mpsc::UnboundedSender<Result<Bytes, HostError>>;

fn input_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    tx: Chunks,
) -> Result<cpal::Stream, HostError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let rate = config.sample_rate;
    let channels = config.channels;
    let errors = tx.clone();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _| {
                let samples: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
                let mono = resample(&downmix(&samples, channels), rate, RATE);
                let _ = tx.send(Ok(Bytes::from(f32_to_pcm16(&mono))));
            },
            move |e| {
                let _ = errors.send(Err(device_error(e)));
            },
            None,
        )
        .map_err(device_error)
}

fn open_input(tx: Chunks) -> Result<cpal::Stream, HostError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| HostError::Io("audio: no microphone".into()))?;
    let supported = device.default_input_config().map_err(device_error)?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => input_stream::<f32>(&device, config, tx)?,
        SampleFormat::I16 => input_stream::<i16>(&device, config, tx)?,
        SampleFormat::U16 => input_stream::<u16>(&device, config, tx)?,
        other => {
            return Err(HostError::Io(format!(
                "audio: unsupported microphone format {other:?}"
            )));
        }
    };
    stream.play().map_err(device_error)?;
    Ok(stream)
}

harness_async_trait! {
    impl Microphone for CpalMicrophone {
        async fn capture(&self, format: AudioFormat) -> Result<AudioStream, HostError> {
            require_pcm24k(format)?;
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let (ready, opened) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || match open_input(tx.clone()) {
                Ok(stream) => {
                    let _ = ready.send(Ok(()));
                    // Capture until the caller drops the stream.
                    while !tx.is_closed() {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready.send(Err(e));
                }
            });
            opened.await.map_err(|_| HostError::Io("audio: the capture thread died".into()))??;
            Ok(boxed_stream(futures::stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|item| (item, rx))
            })))
        }
    }
}

type Queue = Arc<Mutex<VecDeque<f32>>>;

fn output_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    queue: Queue,
) -> Result<cpal::Stream, HostError>
where
    T: SizedSample + FromSample<f32>,
{
    device
        .build_output_stream::<T, _, _>(
            config,
            move |out: &mut [T], _| {
                let mut q = queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for slot in out.iter_mut() {
                    *slot = T::from_sample(q.pop_front().unwrap_or(0.0));
                }
            },
            |e| tracing::warn!(error = %e, "audio output"),
            None,
        )
        .map_err(device_error)
}

fn open_output(queue: Queue) -> Result<(cpal::Stream, u32, u16), HostError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| HostError::Io("audio: no speaker".into()))?;
    let supported = device.default_output_config().map_err(device_error)?;
    let config = supported.config();
    let (rate, channels) = (config.sample_rate, config.channels);
    let stream = match supported.sample_format() {
        SampleFormat::F32 => output_stream::<f32>(&device, config, queue)?,
        SampleFormat::I16 => output_stream::<i16>(&device, config, queue)?,
        SampleFormat::U16 => output_stream::<u16>(&device, config, queue)?,
        other => {
            return Err(HostError::Io(format!(
                "audio: unsupported speaker format {other:?}"
            )));
        }
    };
    stream.play().map_err(device_error)?;
    Ok((stream, rate, channels))
}

harness_async_trait! {
    impl Speaker for CpalSpeaker {
        async fn play(&self, format: AudioFormat, mut audio: AudioStream) -> Result<(), HostError> {
            require_pcm24k(format)?;
            let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
            let done = Arc::new(AtomicBool::new(false));
            let (ready, opened) = tokio::sync::oneshot::channel();
            {
                let queue = Arc::clone(&queue);
                let done = Arc::clone(&done);
                std::thread::spawn(move || match open_output(queue) {
                    Ok((stream, rate, channels)) => {
                        let _ = ready.send(Ok((rate, channels)));
                        while !done.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready.send(Err(e));
                    }
                });
            }
            let (rate, channels) = opened.await.map_err(|_| HostError::Io("audio: the playback thread died".into()))??;
            let result = async {
                while let Some(chunk) = audio.next().await {
                    let samples = upmix(&resample(&pcm16_to_f32(&chunk?), RATE, rate), channels);
                    queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(samples);
                }
                // Let what is queued play out.
                while !queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok(())
            }
            .await;
            done.store(true, Ordering::Relaxed);
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_round_trips_and_clips() {
        let samples = [0.0, 0.5, -0.5, 1.5, -1.5];
        let back = pcm16_to_f32(&f32_to_pcm16(&samples));
        assert!((back[1] - 0.5).abs() < 1e-3 && (back[2] + 0.5).abs() < 1e-3);
        assert!((back[3] - 1.0).abs() < 1e-3, "clipped high");
        assert!((back[4] + 1.0).abs() < 1e-3, "clipped low");
    }

    #[test]
    fn resampling_keeps_the_duration_and_interpolates() {
        let one_second_48k: Vec<f32> = (0..48_000u16).map(|i| f32::from(i % 2)).collect();
        assert_eq!(resample(&one_second_48k, 48_000, 24_000).len(), 24_000);
        assert_eq!(resample(&[0.0, 1.0], 24_000, 48_000), [0.0, 0.5, 1.0, 1.0]);
        assert_eq!(resample(&[0.25], 16_000, 16_000), [0.25]);
        assert_eq!(resample(&[], 48_000, 24_000), Vec::<f32>::new());
    }

    #[test]
    fn channels_mix_down_and_up() {
        assert_eq!(downmix(&[0.0, 1.0, 0.5, 0.5], 2), [0.5, 0.5]);
        assert_eq!(upmix(&[0.25, 0.75], 2), [0.25, 0.25, 0.75, 0.75]);
        assert_eq!(downmix(&[0.3], 1), [0.3]);
    }

    #[tokio::test]
    async fn another_format_is_refused_before_a_device_is_opened() {
        let err = CpalMicrophone
            .capture(AudioFormat::Opus)
            .await
            .err()
            .unwrap();
        assert!(matches!(err, HostError::Denied(_)), "{err}");
    }
}
