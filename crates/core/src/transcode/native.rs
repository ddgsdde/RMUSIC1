//! Preserve FLAC's declared programme boundary before re-encoding. Encoder EOF
//! is successful only after the strict decoder has supplied every declared sample.
use rmusic_audio::{decoder::{DecoderAbort, FfmpegDecoder, PcmEncoding}, dsp::DspProcessor, fade::FadeConfig};
use std::io;
use tokio::{io::AsyncWriteExt, process::ChildStdin, sync::mpsc, task::JoinHandle};
use tokio::sync::broadcast;
use crate::loudness::Measurement;
pub(super) struct Input {
    decoder: FfmpegDecoder,
    pub abort: DecoderAbort,
    channels: u16,
    render: Option<Render>,
}
struct Render {
    dsp: DspProcessor,
    fade: FadeConfig,
    rate: u32,
    duration: Option<f64>,
    frame: u64,
    discard_before: u64,
    samples: Vec<f64>,
    loudness: Option<(String, broadcast::Receiver<(String, Measurement)>)>,
}

impl Render {
    fn process(&mut self, bytes: &mut Vec<u8>, channels: usize) -> io::Result<()> {
        if let Some((track, updates)) = &mut self.loudness {
            use broadcast::error::TryRecvError;
            loop {
                match updates.try_recv() {
                    Ok((id, result)) if &id == track => {
                        if self.frame <= self.discard_before {
                            self.dsp.prime_loudness(result.integrated, result.true_peak);
                        } else {
                            self.dsp.smooth_loudness(result.integrated, result.true_peak);
                        }
                        self.loudness = None;
                        break;
                    }
                    Ok(_) | Err(TryRecvError::Lagged(_)) => continue,
                    Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                }
            }
        }
        self.samples.clear();
        self.samples.extend(bytes.chunks_exact(8).map(|sample| f64::from_le_bytes(sample.try_into().unwrap())));
        let frame = self.frame;
        let rate = f64::from(self.rate);
        let fade = &self.fade;
        let duration = self.duration;
        self.dsp.process_enveloped(&mut self.samples, |index| fade.gain((frame + index as u64) as f64 / rate, duration))
            .map_err(io::Error::other)?;
        let frames = (self.samples.len() / channels) as u64;
        let discard = self.discard_before.saturating_sub(frame).min(frames) as usize;
        bytes.clear();
        bytes.extend(self.samples[discard * channels..].iter().flat_map(|sample| sample.to_le_bytes()));
        self.frame += frames;
        Ok(())
    }
}

#[cfg(test)]
mod live_loudness_tests {
    use super::*;
    use rmusic_audio::dsp::DspConfig;

    #[test]
    fn live_loudness_ramps_the_current_stream_and_primes_an_unheard_successor() {
        let (updates,_) = broadcast::channel(8);
        let render = |name: &str| Render {
            dsp: DspProcessor::new(DspConfig { enabled:true, normalize:true, target_lufs:-14., ..Default::default() },8000,1,None).unwrap(),
            fade:Default::default(), rate:8000, duration:Some(10.), frame:0, discard_before:0, samples:Vec::new(),
            loudness:Some((name.to_owned(),updates.subscribe())),
        };
        let mut current=render("current");
        let mut next=render("next");
        let mut first=0.2f64.to_le_bytes().to_vec();
        current.process(&mut first,1).unwrap();
        assert_eq!(first,0.2f64.to_le_bytes());
        updates.send(("next".into(),Measurement {integrated:-24.,true_peak:Some(-2.)})).unwrap();
        updates.send(("current".into(),Measurement {integrated:-24.,true_peak:Some(-2.)})).unwrap();
        let mut ramp=0.2f64.to_le_bytes().repeat(4000);
        current.process(&mut ramp,1).unwrap();
        let sample=|bytes:&[u8]|f64::from_le_bytes(bytes[..8].try_into().unwrap());
        let expected=0.2*10f64.powf(2./20.);
        assert!((sample(&ramp)-0.2).abs()<0.0001);
        assert!((sample(&ramp[ramp.len()-8..])-expected).abs()<0.000001);
        assert_eq!(current.frame,4001);
        next.process(&mut first,1).unwrap();
        assert!((sample(&first)-expected).abs()<0.000001);
    }
}
impl Input {
    pub async fn open(
        url: &str,
        start: f64,
        rate: u32,
        channels: u16,
    ) -> anyhow::Result<Option<Self>> {
        let metadata = rmusic_audio::probe::probe_async(std::path::Path::new(url)).await?;
        let Some(samples) = metadata.flac_samples else {
            return Ok(None);
        };
        let decoder = FfmpegDecoder::open_flac(
            url,
            start,
            rate,
            channels,
            PcmEncoding::Signed32,
            Some((samples, metadata.sample_rate)),
        )?;
        let abort = decoder.abort_handle();
        Ok(Some(Self {
            decoder,
            abort,
            channels,
            render: None,
        }))
    }
    pub async fn render(
        state: &crate::state::AppState, url: &str, track: &crate::models::Track, start: f64,
        rate: u32, channels: u16, recipe: crate::render_media::Recipe,
        loudness: Option<broadcast::Receiver<(String, Measurement)>>,
    ) -> crate::error::ApiResult<Self> {
        use crate::error::ApiError;
        let metadata = rmusic_audio::probe::probe_async(std::path::Path::new(url)).await?;
        if metadata.is_dsd() {
            return Err(ApiError::bad("Core 音区 DSP 当前仅处理 PCM；DSD 原样输出保持旁路"));
        }
        let duration = metadata.duration.or_else(|| (track.duration > 0.).then_some(track.duration));
        let mut config = recipe.dsp;
        if !config.enabled { config = Default::default(); }
        // Allow fades without requiring EQ or normalization to be switched on.
        config.enabled = true;
        let filtered = config.eq_stages().any(|(bypass, bands)| !bypass && bands.iter().any(|band| band.enabled));
        let crossfeed = config.stereo.as_ref().is_some_and(|stereo| stereo.crossfeed > 0.);
        let mut history = if filtered { rate as u64 * 2 } else if crossfeed { rate as u64 / 10 } else { 0 };
        let measured = config.peak_limited_measurement(track.integrated_lufs, track.provider_data["analysis"]["loudness"]["true_peak_dbfs"].as_f64());
        let mut dsp = DspProcessor::new(config.clone(), rate, channels as usize, measured)
            .map_err(|error| ApiError::bad(error.to_string()))?;
        if let Some(selection) = &config.convolution {
            let response = crate::impulse_assets::render_response(state, selection, rate, channels).await?;
            history = history.max(response.frames().saturating_sub(1) as u64);
            dsp.attach_convolution(rate, response.kernels().to_vec())
                .map_err(|error| ApiError::bad(error.to_string()))?;
        }
        let discard_before = (start * rate as f64).round() as u64;
        let frame = discard_before.saturating_sub(history);
        let extent = metadata.flac_samples.map(|samples| (samples, metadata.sample_rate));
        let decoder = FfmpegDecoder::open_flac(url, frame as f64 / rate as f64, rate, channels, PcmEncoding::Float64, extent)?;
        let abort = decoder.abort_handle();
        Ok(Self {
            decoder, abort, channels,
            render: Some(Render { dsp, fade: recipe.fade, rate, duration, frame, discard_before, samples: Vec::new(),
                loudness: loudness.filter(|_| measured.is_none()).map(|updates| (track.id.clone(), updates)) }),
        })
    }

    pub fn sample_format(&self) -> &'static str {
        if self.render.is_some() { "f64le" } else { "s32le" }
    }

    pub fn feed(mut self, mut stdin: ChildStdin) -> JoinHandle<io::Result<()>> {
        let (sender, mut receiver) = mpsc::channel::<io::Result<Vec<u8>>>(2);
        let abort = self.abort.clone();
        let producer = tokio::task::spawn_blocking(move || {
            let channels = self.channels as usize;
            let width = channels * if self.render.is_some() { 8 } else { 4 };
            loop {
                let mut bytes = vec![0; 4096 * width];
                let result = self.decoder.read_frames(&mut bytes, width);
                match result {
                    Ok(0) => return Ok(()),
                    Ok(frames) => {
                        bytes.truncate(frames * width);
                        if let Some(render) = self.render.as_mut() {
                            if let Err(error) = render.process(&mut bytes, channels) {
                                let _ = sender.blocking_send(Err(error));
                                return Err(io::Error::other("Core DSP failed"));
                            }
                        }
                        if bytes.is_empty() { continue; }
                        if sender.blocking_send(Ok(bytes)).is_err() {
                            return Err(io::Error::other("stream consumer closed"));
                        }
                    }
                    Err(_) => {
                        let _ = sender.blocking_send(Err(io::Error::other(
                            "FLAC programme data is incomplete or corrupt",
                        )));
                        return Err(io::Error::other(
                            "FLAC programme data is incomplete or corrupt",
                        ));
                    }
                }
            }
        });
        tokio::spawn(async move {
            let result = async {
                while let Some(bytes) = receiver.recv().await {
                    stdin.write_all(&bytes?).await?;
                }
                stdin.shutdown().await
            }
            .await;
            if result.is_err() {
                abort.abort();
            }
            drop(receiver);
            drop(stdin);
            let decoded = producer.await.map_err(io::Error::other)?;
            result.and(decoded)
        })
    }
}

/// Ogg/FLAC's identification packet includes native STREAMINFO. Never confirm
/// command-line intent if an older encoder silently chose different precision.
pub(super) fn confirmed_header(
    bytes: &[u8],
    rate: u32,
    bits: u16,
    channels: u16,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        bytes.len() <= 65536,
        "FLAC identification exceeds the bounded preface"
    );
    if bytes.len() < 4 {
        return Ok(false);
    }
    anyhow::ensure!(
        bytes.starts_with(b"OggS"),
        "encoder did not produce Ogg/FLAC"
    );
    let Some(marker) = bytes.windows(4).position(|v| v == b"fLaC") else {
        return Ok(false);
    };
    if bytes.len() < marker + 42 {
        return Ok(false);
    }
    anyhow::ensure!(
        bytes[marker + 4] & 0x7f == 0 && bytes[marker + 5..marker + 8] == [0, 0, 34],
        "missing FLAC STREAMINFO"
    );
    let packed = u64::from_be_bytes(bytes[marker + 18..marker + 26].try_into().unwrap());
    let actual_rate = (packed >> 44) as u32;
    let actual_channels = ((packed >> 41) & 7) as u16 + 1;
    let actual_bits = ((packed >> 36) & 31) as u16 + 1;
    anyhow::ensure!(
        (actual_rate, actual_bits, actual_channels) == (rate, bits, channels),
        "encoder cannot provide requested FLAC precision"
    );
    Ok(true)
}

pub(super) fn confirmed_opus_header(bytes:&[u8],channels:u16)->anyhow::Result<bool>{
    anyhow::ensure!(bytes.len()<=65536,"Opus identification exceeds limit");
    if bytes.len()<27{return Ok(false);}
    anyhow::ensure!(bytes.starts_with(b"OggS")&&bytes[4]==0,"encoder did not produce Ogg/Opus");
    let body=27+bytes[26] as usize;
    if bytes.len()<body+19{return Ok(false);}
    anyhow::ensure!(&bytes[body..body+8]==b"OpusHead"&&bytes[body+8]<=15&&bytes[body+9] as u16==channels,"encoder did not provide requested Opus channels");
    Ok(true)
}
