// SPDX-License-Identifier: GPL-3.0-or-later
//! Turning a sound file into samples.
//!
//! The sound theme spec allows exactly three extensions: `.oga` and `.ogg`, which are Ogg Vorbis,
//! and `.wav`. Everything is decoded up front into interleaved 32-bit floats at the file's own
//! rate; PipeWire resamples on the way to the device. System sounds are short -- the longest
//! IRIX one is under six seconds -- so holding them decoded is a few megabytes at most, and it
//! means a sound starts the moment it is asked for.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// A decoded sound.
#[derive(Debug)]
pub struct Pcm {
    /// Interleaved, `channels` samples per frame, in -1.0..=1.0.
    pub samples: Vec<f32>,
    pub rate: u32,
    /// 1 or 2. Anything wider is mixed down to stereo, since that is the only output profile the
    /// theme lookup asks for.
    pub channels: u32,
}

/// Decode `path`, by its extension.
pub fn decode(path: &Path) -> Result<Pcm, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    let pcm = match extension {
        "oga" | "ogg" => vorbis(path),
        "wav" => wav(path),
        other => Err(format!("unsupported sound type .{other}")),
    }
    .map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(fold_to_stereo(pcm))
}

fn vorbis(path: &Path) -> Result<Pcm, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let mut reader = lewton::inside_ogg::OggStreamReader::new(BufReader::new(file))
        .map_err(|err| err.to_string())?;
    let rate = reader.ident_hdr.audio_sample_rate;
    let channels = u32::from(reader.ident_hdr.audio_channels);
    let mut samples = Vec::new();
    while let Some(packet) = reader
        .read_dec_packet_itl()
        .map_err(|err| err.to_string())?
    {
        samples.extend(packet.into_iter().map(|sample| f32::from(sample) / 32768.0));
    }
    Ok(Pcm {
        samples,
        rate,
        channels,
    })
}

fn wav(path: &Path) -> Result<Pcm, String> {
    let mut reader = hound::WavReader::open(path).map_err(|err| err.to_string())?;
    let spec = reader.spec();
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?,
        hound::SampleFormat::Int => {
            let scale = (1_i64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|sample| sample as f32 / scale))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| err.to_string())?
        }
    };
    Ok(Pcm {
        samples,
        rate: spec.sample_rate,
        channels: u32::from(spec.channels),
    })
}

/// Mix anything wider than stereo down to two channels: odd channels left, even ones right.
///
/// Crude, but a system sound in surround is something no theme ships in its stereo directory;
/// this is here so one that does plays as something rather than as noise.
fn fold_to_stereo(pcm: Pcm) -> Pcm {
    if pcm.channels <= 2 {
        return pcm;
    }
    let width = pcm.channels as usize;
    let samples = pcm
        .samples
        .chunks_exact(width)
        .flat_map(|frame| {
            let (mut left, mut right) = (0.0, 0.0);
            for (index, sample) in frame.iter().enumerate() {
                if index % 2 == 0 {
                    left += sample;
                } else {
                    right += sample;
                }
            }
            let share = (width / 2) as f32;
            [left / width.div_ceil(2) as f32, right / share]
        })
        .collect();
    Pcm {
        samples,
        rate: pcm.rate,
        channels: 2,
    }
}

impl Pcm {
    /// How many frames long the sound is.
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_is_decoded_to_floats() {
        let path =
            std::env::temp_dir().join(format!("wlrix-sound-test-{}.wav", std::process::id()));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 22050,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for sample in [0_i16, i16::MAX, i16::MIN, 16384] {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        let pcm = decode(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(pcm.rate, 22050);
        assert_eq!(pcm.channels, 2);
        assert_eq!(pcm.frames(), 2);
        assert_eq!(pcm.samples[0], 0.0);
        assert!((pcm.samples[1] - 1.0).abs() < 0.001);
        assert_eq!(pcm.samples[2], -1.0);
        assert_eq!(pcm.samples[3], 0.5);
    }

    #[test]
    fn surround_is_folded_to_stereo() {
        let pcm = fold_to_stereo(Pcm {
            samples: vec![1.0, 0.0, 1.0, 0.0],
            rate: 48000,
            channels: 4,
        });
        assert_eq!(pcm.channels, 2);
        assert_eq!(pcm.samples, [1.0, 0.0]);
    }

    #[test]
    fn an_unknown_extension_is_refused() {
        assert!(decode(Path::new("/nonexistent/bell.mp3")).is_err());
    }
}
