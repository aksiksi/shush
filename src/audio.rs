use std::time::Duration;

use super::error::Result;

// Converts a timestamp in time base units into a [std::time::Duration] that
// represents the timestamp in time units.
#[allow(dead_code)]
pub(crate) fn to_timestamp(
    time_base: ffmpeg_next::util::rational::Rational,
    raw_timestamp: i64,
) -> Duration {
    let time_base: f64 = time_base.into();
    let ts = raw_timestamp as f64 * time_base;
    Duration::from_secs_f64(ts)
}

// Seeks the video stream to the given timestamp. Under the hood, this uses
// the standard FFmpeg function, `avformat_seek_file`.
pub(crate) fn seek_to_timestamp(
    ctx: &mut ffmpeg_next::format::context::Input,
    time_base: ffmpeg_next::util::rational::Rational,
    timestamp: Duration,
) -> Result<()> {
    let min_timestamp = timestamp - Duration::from_millis(1000);
    let max_timestamp = timestamp + Duration::from_millis(1000);

    let time_base: f64 = time_base.into();
    let duration = Duration::from_millis((ctx.duration() as f64 * time_base) as u64);
    // TODO(aksiksi): Make this an error.
    assert!(
        max_timestamp < duration,
        "timestamp must be less than the stream duration"
    );

    // Convert timestamps from ms to seconds, then divide by time_base to get each timestamp
    // in time_base units.
    let timestamp = (timestamp.as_millis() as f64 / time_base) as i64;
    let min_timestamp = (min_timestamp.as_millis() as f64 / time_base) as i64;
    let max_timestamp = (max_timestamp.as_millis() as f64 / time_base) as i64;

    Ok(ctx.seek(timestamp, min_timestamp..max_timestamp)?)
}

/// Finds the "best" audio stream in the given input.
pub(crate) fn find_best_stream(
    input: &ffmpeg_next::format::context::Input,
) -> ffmpeg_next::format::stream::Stream {
    input
        .streams()
        .best(ffmpeg_next::media::Type::Audio)
        .expect("unable to find an audio stream")
}

/// Thin wrapper around the native `FFmpeg` audio decoder.
struct Decoder {
    decoder: ffmpeg_next::codec::decoder::Audio,
}

impl Decoder {
    fn build_threading_config() -> ffmpeg_next::codec::threading::Config {
        let mut config = ffmpeg_next::codec::threading::Config::default();
        config.count = std::thread::available_parallelism()
            .expect("unable to determine available parallelism")
            .get();
        config.kind = ffmpeg_next::codec::threading::Type::Frame;
        config
    }

    fn from_stream(stream: ffmpeg_next::format::stream::Stream, threaded: bool) -> Result<Self> {
        let ctx = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())?;
        let mut decoder = ctx.decoder();
        if threaded {
            decoder.set_threading(Self::build_threading_config());
        }
        let decoder = decoder.audio()?;
        Ok(Self { decoder })
    }

    fn send_packet(&mut self, packet: &ffmpeg_next::packet::Packet) -> Result<()> {
        Ok(self.decoder.send_packet(packet)?)
    }

    fn receive_frame(&mut self, frame: &mut ffmpeg_next::frame::Audio) -> Result<()> {
        Ok(self.decoder.receive_frame(frame)?)
    }
}

pub fn decode(
    ctx: &mut ffmpeg_next::format::context::Input,
    stream_idx: usize,
    duration: Option<Duration>,
    seek_to: Option<Duration>,
    target_sample_rate: u32,
    threaded: bool,
) -> Result<Vec<f32>> {
    let stream = ctx.stream(stream_idx).unwrap();
    let time_base = stream.time_base();
    let mut decoder = Decoder::from_stream(stream, threaded).unwrap();

    let mut data = Vec::new();
    let mut frame = ffmpeg_next::frame::Audio::empty();
    let mut frame_resampled = ffmpeg_next::frame::Audio::empty();

    // Setup the audio resampler
    let mut resampler = decoder
        .decoder
        .resampler(
            ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
            ffmpeg_next::ChannelLayout::MONO,
            target_sample_rate,
        )
        .unwrap();

    // If required, seek to the given position in the stream.
    if let Some(seek_to) = seek_to {
        seek_to_timestamp(ctx, time_base, seek_to)?;
    }

    // Compute the end timestamp in time base units. This allows for quick
    // comparison with the PTS.
    let end_timestamp = duration.map(|d| {
        let d = seek_to.unwrap_or(Duration::ZERO) + d;
        (d.as_secs_f64() / f64::from(time_base)) as i64
    });

    // Build an iterator over packets in the stream.
    //
    // We are only interested in packets for the selected stream.
    // We also only we want to consider packets as long as we haven't reached
    // the target end_timestamp.
    let audio_packets = ctx
        .packets()
        .filter(|(s, _)| s.index() == stream_idx)
        .map(|(_, p)| p)
        .take_while(|p| {
            if end_timestamp.is_none() {
                true
            } else {
                p.pts().unwrap() < end_timestamp.unwrap()
            }
        });

    for p in audio_packets {
        if p.pts().unwrap() <= 0 {
            // Skip packets with an invalid PTS. This can happen if, e.g., the
            // video was trimmed.
            // See: https://stackoverflow.com/a/41032346/845275
            continue;
        }

        decoder.send_packet(&p).unwrap();
        while decoder.receive_frame(&mut frame).is_ok() {
            // Resample the frame to S16 stereo and return the frame delay.
            let mut delay = match resampler.run(&frame, &mut frame_resampled) {
                Ok(v) => v,
                // If resampling fails due to changed input, construct a new local resampler for this frame
                // and swap out the global resampler.
                Err(ffmpeg_next::Error::InputChanged) => {
                    let mut local_resampler = frame
                        .resampler(
                            ffmpeg_next::format::Sample::F32(
                                ffmpeg_next::format::sample::Type::Packed,
                            ),
                            ffmpeg_next::ChannelLayout::MONO,
                            target_sample_rate,
                        )
                        .unwrap();
                    let delay = local_resampler
                        .run(&frame, &mut frame_resampled)
                        .expect("failed to resample frame");

                    resampler = local_resampler;

                    delay
                }
                // We don't expect any other errors to occur.
                Err(_) => unreachable!("unexpected error"),
            };

            loop {
                // Obtain a slice of raw bytes in interleaved format.
                // We have two channels, so the bytes look like this: c1, c1, c2, c2, c1, c1, c2, c2, ...
                //
                // Note that `data` is a fixed-size buffer. To get the _actual_ sample bytes, we need to use:
                // a) sample count, b) channel count, and c) number of bytes per F32 sample.
                let raw_samples = &frame_resampled.data(0)[..frame_resampled.samples()
                    * frame_resampled.channels() as usize
                    * std::mem::size_of::<f32>()];

                // Transmute the raw byte slice into a slice of i16 samples.
                // This looks like: c1, c2, c1, c2, ...
                //
                // SAFETY: We know for a fact that the returned buffer contains i16 samples
                // because we explicitly told the resampler to return S16 samples (see above).
                let (_, samples, _) = unsafe { raw_samples.align_to() };

                data.extend(samples);

                if delay.is_none() {
                    break;
                } else {
                    delay = resampler.flush(&mut frame_resampled).unwrap();
                }
            }
        }
    }

    Ok(data)
}
