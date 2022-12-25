mod audio;
mod error;

use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, Parser};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

use crate::error::{Error, Result};

// Reference: https://github.com/ggerganov/whisper.cpp/blob/ac521a566ea6a79ba968c30101140db9f65d187b/whisper.h#L21
const WHISPER_SAMPLE_RATE: u32 = 16000; // Hz

#[derive(Debug, Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(help = "Path to video or audio file.")]
    path: String,

    #[clap(required = true, long, help = "Path to the Whisper model.")]
    model: String,

    #[clap(
        long,
        default_value = "false",
        action(ArgAction::SetTrue),
        help = "Enable multi-threaded decoding in FFmpeg."
    )]
    threaded_decoding: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    ffmpeg_next::init()?;
    ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Fatal);

    let path: PathBuf = cli.path.into();

    // Decode and resample audio into raw F32 PCM samples with the target sample rate.
    let mut ctx = ffmpeg_next::format::input(&path)?;
    let stream = audio::find_best_stream(&ctx);
    let stream_idx = stream.index();
    let data = audio::decode(
        &mut ctx,
        stream_idx,
        None,
        None,
        WHISPER_SAMPLE_RATE,
        cli.threaded_decoding,
    )?;

    // Load the model.
    let mut ctx = match WhisperContext::new(&cli.model) {
        Ok(ctx) => ctx,
        Err(e) => Err(Error::from(e))?,
    };

    let mut params = FullParams::new(SamplingStrategy::Greedy { n_past: 0 });
    if ctx.is_multilingual() {
        params.set_language("en");
        params.set_translate(true);
    }

    match ctx.full(params, &data[..]) {
        Ok(_) => (),
        Err(e) => Err(Error::from(e))?,
    };

    let num_segments = ctx.full_n_segments();
    for i in 0..num_segments {
        let segment = ctx.full_get_segment_text(i).expect("failed to get segment");
        let start_timestamp = Duration::from_secs((ctx.full_get_segment_t0(i) / 100) as u64);
        let end_timestamp = Duration::from_secs((ctx.full_get_segment_t1(i) / 100) as u64);
        print!("{}", format_srt(&segment, (i+1) as usize, start_timestamp, end_timestamp));
    }

    Ok(())
}

fn to_timestamp(ts: Duration, comma: bool) -> String {
    let sep = if comma { "," } else { "." };
    let mut msec = ts.as_millis() as u64;
    let hr = msec / (1000 * 60 * 60);
    msec -= hr * 1000 * 60 * 60;
    let min = msec / (1000 * 60);
    msec -= min * 1000 * 60;
    let sec = msec / 1000;
    msec -= sec * 1000;
    format!("{:02}:{:02}:{:02}{}{:03}", hr, min, sec, sep, msec)
}

fn format_srt(text: &str, idx: usize, start_ts: Duration, end_ts: Duration) -> String {
    format!(
        "{}\n{} --> {}\n{}\n\n",
        idx,
        to_timestamp(start_ts, true),
        to_timestamp(end_ts, true),
        text
    )
}
