use super::sequence::{DeviceClock, PcmStream, Read};
use super::*;
use anyhow::Context;
use wasapi::*;

pub(super) fn path(
    request: &PlaybackRequest,
    format: SequenceFormat,
    name: &str,
    mixer: Option<u32>,
    volume: f64,
) -> SignalPath {
    if request.is_dsd() {
        if format.bit_depth==1 {
            return SignalPath{output:name.into(),sample_rate:format.sample_rate*8,channels:format.channels,exclusive:format.exclusive,
                bit_depth:1,sample_type:"dsd_native".into(),mixer_sample_rate:None,bit_perfect_eligible:format.exclusive&&!format.float_output&&volume==1.,
                processing:vec![format!("DSD {} Hz → ASIO native DSD; no PCM conversion",format.sample_rate*8)]};
        }
        return SignalPath{output:name.into(),sample_rate:format.sample_rate,channels:format.channels,exclusive:format.exclusive,
            bit_depth:format.bit_depth,sample_type:"dsd_dop".into(),mixer_sample_rate:None,
            bit_perfect_eligible:format.exclusive&&!format.float_output&&format.bit_depth>=24&&volume==1.,
            processing:vec![format!("DSD {} Hz → DoP {} Hz; no PCM conversion",request.sample_rate as u64*16,format.sample_rate)]};
    }
    let mut processing = Vec::new();
    if request.dsp.enabled {
        processing.push(
            if format.float_output {
                if format.bit_depth==64 {"DSP f64 → float64 PCM"}else{"DSP f64 → float32 PCM"}
            } else {
                "DSP f64 → integer PCM"
            }
            .into(),
        );
    }
    if request.transitions.enabled() {
        processing.push("Fade envelope".into());
    }
    if request.dsp.enabled {if let Some(impulse)=request.impulse.as_ref().and_then(|bank|bank.response(format.sample_rate).ok()) {processing.push(format!("FIR f64: {} taps @ {} Hz; original gain",impulse.frames(),impulse.sample_rate()));}}
    if request.dsp.normalize && request.measured_lufs.is_none() {
        processing.push("Normalization pending measured LUFS".into());
    }
    if !format.exclusive {
        processing.push("Windows shared mixer / compatible rate conversion".into());
    }
    if request.sample_rate != format.sample_rate {
        processing.push(format!(
            "FFmpeg rate conversion {} → {} Hz",
            request.sample_rate, format.sample_rate
        ));
    }
    if request.channels != format.channels {
        processing.push(format!(
            "FFmpeg channel conversion {} → {}",
            request.channels, format.channels
        ));
    }
    if !request.source_integer_lossless {
        processing.push("Decoded source to PCM".into());
    }
    if format.float_output && !request.dsp.enabled && request.source_bit_depth > 24 {
        processing.push("Float32 PCM precision conversion".into());
    }
    if volume != 1. {
        processing.push("Software volume".into());
    }
    SignalPath {
        output: name.into(),
        sample_rate: format.sample_rate,
        channels: format.channels,
        exclusive: format.exclusive,
        bit_depth: format.bit_depth,
        sample_type: if format.float_output {
            "float"
        } else {
            "integer"
        }
        .into(),
        mixer_sample_rate: mixer,
        bit_perfect_eligible: format.exclusive
            && !format.float_output
            && !request.dsp.enabled
            && !request.transitions.enabled()
            && volume == 1.
            && request.source_integer_lossless
            && request.sample_rate == format.sample_rate
            && request.channels == format.channels
            && request.source_bit_depth > 0
            && request.source_bit_depth <= format.bit_depth,
        processing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wasapi_format_signal_path_uses_effective_precision_and_conservative_eligibility() {
        let mut request = PlaybackRequest {flac_samples: None, dsd_pcm: None, input: String::new(), position: 0., sample_rate: 768000,
            channels: 2, source_integer_lossless: true, source_bit_depth: 24, dsp: DspConfig::default(),
            impulse: None, measured_lufs: None, volume: 1., duration: None, transitions: Default::default()};
        let format = SequenceFormat {sample_rate: 768000, channels: 2, channel_mask: 3, exclusive: true,
            float_output: false, bit_depth: 24};
        let signal = path(&request, format, "test", None, 1.);
        assert_eq!(signal.bit_depth, 24);
        assert!(signal.bit_perfect_eligible);
        for bits in [0, 32] {request.source_bit_depth = bits;assert!(!path(&request, format, "test", None, 1.).bit_perfect_eligible);}
        request.source_bit_depth = 24;
        assert!(!path(&request, format, "test", None, 0.5).bit_perfect_eligible);
        assert!(!path(&request, SequenceFormat {exclusive: false, ..format}, "test", Some(192000), 1.).bit_perfect_eligible);
    }
}
/// One WASAPI client for the entire sequence. Next sources must be reserved before current EOF.
pub fn play_sequence(
    request: PlaybackRequest,
    config: OutputConfig,
    control: Arc<PlaybackControl>,
    sequence: PlaybackSequence,
    mut update: impl FnMut(PlaybackUpdate),
) -> Result<()> {
    let scheduled=control.start.setup(request.is_dsd())?;
    request.validate_dop(&config)?;
    let dop=request.is_dsd();
    let _run = sequence.guard()?;
    initialize_mta().ok()?;
    if !(20..=2000).contains(&config.buffer_ms) {
        bail!("buffer must be 20..2000 ms");
    }
    control.initialize_volume(request.volume)?;
    let enumerator = DeviceEnumerator::new()?;
    let device = match &config.device_id {
        Some(id) => enumerator.get_device(id)?,
        None => enumerator.get_default_device(&Direction::Render)?,
    };
    let name = device.get_friendlyname()?;
    let mut client = device.get_iaudioclient()?;
    let target=config.desired_format(&request)?;
    let float_output=target.float_output;
    let sample_type = if float_output {
        SampleType::Float
    } else {
        SampleType::Int
    };
    let desired = WaveFormat::new(
        32,
        target.bit_depth as usize,
        &sample_type,
        target.sample_rate as usize,
        target.channels as usize,
        None,
    );
    let (format, mode) = if config.exclusive {
        let mut supported=None;
        for rate in if dop{vec![request.sample_rate]}else{config.limits.rates(request.sample_rate)} {
            let candidate=WaveFormat::new(32,target.bit_depth as usize,&sample_type,rate as usize,target.channels as usize,None);
            if let Ok(format)=wasapi_format::limited(&candidate,if dop{24}else{request.source_bit_depth},config.limits.max_bit_depth,|format|client.is_supported_exclusive_with_quirks(format)){supported=Some(format);break;}
        }
        let supported=supported.context("设备没有符合采样率／位深上限的独占 PCM 格式")?;
        let (_, minimum) = client.get_device_period()?;
        let period = client.calculate_aligned_period_near(
            (config.buffer_ms as i64 * 10000).max(minimum),
            None,
            &supported,
        )?;
        (
            supported,
            StreamMode::EventsExclusive { period_hns: period },
        )
    } else {
        (
            desired,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: config.buffer_ms as i64 * 10000,
            },
        )
    };
    client.initialize_client(&format, &Direction::Render, &mode)?;
    config.resync(&control)?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;
    let clock = client.get_audioclock()?;
    let frequency = clock.get_frequency()?;
    let feedback=clock::Session::new(control.clone());
    let output_format = SequenceFormat {
        sample_rate: format.get_samplespersec(),
        channels: format.get_nchannels(),
        channel_mask:format.get_dwchannelmask(),
        exclusive: config.exclusive,
        float_output,
        bit_depth: format.get_validbitspersample(),
    };
    control
        .output_sample_rate
        .store(output_format.sample_rate, Ordering::Relaxed);
    let mixer = if config.exclusive {
        None
    } else {
        client.get_mixformat().ok().map(|f| f.get_samplespersec())
    };
    let stream = PcmStream::open(request, sequence.clone(), control.clone(), output_format)?;
    let mut stream = super::buffered::ReadAhead::new(stream,control.clone(),output_format)?;
    let mut dop_encoder=if dop{Some(crate::dsd::DopEncoder::new(output_format.channels)?)}else{None};
    let frame_bytes = output_format.channels as usize * 4;
    let mut packet = Vec::new();
    let mut packet_frames = 0;
    let mut packet_capacity = 0;
    let mut eof = false;
    let mut started = false;
    let mut paused = false;
    let mut last_event = std::time::Instant::now();
    let mut meter = Meter::default();
    let mut timeline = DeviceClock::default();
    let mut extension_wait = None;
    let mut publish = |final_end: bool, meter: Meter, continuity_lost: bool| -> Result<()> {
        let ((id, position, source, ended),output_frames) = sequence.heard_with_frames(&control, final_end)?;
        let mut signal_path = path(&source, output_format, &name, mixer, control.volume());
        config.limits.signal_path(&source,&mut signal_path);
        sequence.mark_rate_adjustment(&mut signal_path);
        if continuity_lost{signal_path.bit_perfect_eligible=false;signal_path.processing.push("WASAPI underrun / continuity lost".into());}
        update(PlaybackUpdate {
            output_frames,
            playback_id: id,
            position,
            ended,
            signal_path,
            meter,
            continuity_lost,
        });
        Ok(())
    };
    let result = (|| -> Result<()> {
        loop {
            if control.stopped.load(Ordering::Acquire) {
                break;
            }
            let wanted = control.paused.load(Ordering::Relaxed);
            if wanted != paused && started {
                if wanted {
                    client.stop_stream()?;
                } else {
                    client.start_stream()?;
                }
                paused = wanted;
                last_event = std::time::Instant::now();
            }
            let ready = !started || wanted || event.wait_for_event(20).is_ok();
            if ready {
                last_event = std::time::Instant::now();
            }
            let before=clock::host_ns();
            let (position, stamp) = clock.get_position()?;
            let after=clock::host_ns();
            let played =
                ((position as u128 * output_format.sample_rate as u128) / frequency as u128) as u64;
            let heard = timeline.advance(
                played,
                control.frames_written.load(Ordering::Relaxed),
                started && !eof && !wanted,
            );
            control.frames_played.store(heard, Ordering::Release);
            let programme=(!dop&&!timeline.continuity_lost&&heard<=control.programme_end.load(Ordering::Acquire)).then_some(clock::Programme{frames:heard,sample_rate:output_format.sample_rate});
            feedback.record_programme(clock::Source::WasapiTicks,position,frequency,stamp.saturating_mul(100),clock::Domain::Qpc,before,after,started&&!wanted,programme);
            control
                .clock_ns
                .store(stamp.saturating_mul(100), Ordering::Relaxed);
            if started {
                publish(false, meter.clone(), timeline.continuity_lost)?;
            }
            if wanted {
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            if eof && packet_frames == 0 {
                if heard == control.frames_written.load(Ordering::Relaxed) {
                    publish(true, meter.clone(), timeline.continuity_lost)?;
                    break;
                }
            }
            if !ready {
                if last_event.elapsed()
                    > std::time::Duration::from_millis(u64::from(
                        config.buffer_ms.saturating_mul(3).max(2000),
                    ))
                {
                    return Err(DeviceUnavailable("WASAPI output stopped signaling device buffers".into()).into());
                }
                continue;
            }
            if eof && packet_frames == 0 {
                continue;
            }
            let space = client.get_available_space_in_frames()? as usize;
            if space == 0 {
                continue;
            }
            if packet_frames == 0 {
                packet.clear();
                packet_capacity = space;
            }
            let mut waiting = false;
            while !eof && packet_frames < packet_capacity {
                match stream.read(packet_capacity - packet_frames) {
                    Ok(Read::Data(chunk)) => {
                        packet.extend_from_slice(&chunk.bytes);
                        packet_frames += chunk.frames;
                        meter = chunk.meter;
                    }
                    Ok(Read::Pending) => {
                        waiting = true;
                        break;
                    }
                    Ok(Read::End) => eof = true,
                    Err(error) if control.stopped.load(Ordering::Acquire) => {
                        let _ = error;
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            if control.stopped.load(Ordering::Acquire) {
                break;
            }
            if control.paused.load(Ordering::Relaxed) {
                continue;
            }
            // A partially filled exclusive packet remains staged while next-track preparation runs.
            if waiting && config.exclusive && packet_frames < packet_capacity {
                let since=extension_wait.get_or_insert_with(std::time::Instant::now);
                if stream.waiting_for_next() && (!started || since.elapsed()>std::time::Duration::from_secs(2)) && sequence.finish_window_if_empty() {
                    timeline.continuity_lost=true;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            extension_wait=None;
            if packet_frames == 0 {
                if waiting {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                continue;
            }
            if packet_frames > space {
                continue;
            }
            let written = if config.exclusive && eof && packet_frames < packet_capacity {
                packet.resize(packet_capacity * frame_bytes, 0);
                packet_capacity
            } else {
                packet_frames
            };
            // Decode/HTTP reads may have stalled since the last clock sample. Account for any
            // idle device frames before scheduling this packet, never after adding its samples.
            let before=clock::host_ns();
            let (position, stamp) = clock.get_position()?;
            let after=clock::host_ns();
            let played =
                ((position as u128 * output_format.sample_rate as u128) / frequency as u128) as u64;
            let heard = timeline.advance(
                played,
                control.frames_written.load(Ordering::Relaxed),
                started,
            );
            control.frames_played.store(heard, Ordering::Release);
            let programme=(!dop&&!timeline.continuity_lost&&heard<=control.programme_end.load(Ordering::Acquire)).then_some(clock::Programme{frames:heard,sample_rate:output_format.sample_rate});
            feedback.record_programme(clock::Source::WasapiTicks,position,frequency,stamp.saturating_mul(100),clock::Domain::Qpc,before,after,started&&!wanted,programme);
            control
                .clock_ns
                .store(stamp.saturating_mul(100), Ordering::Relaxed);
            if let Some(encoder)=dop_encoder.as_mut() {
                if control.volume_setting()!=1. {bail!("原样 DSD 不支持软件音量；请使用 DAC 音量控制");}
                encoder.stamp(&mut packet,crate::dsd::DopContainer::LeftAligned32,packet_frames,control.muted.load(Ordering::Relaxed))?;
            }
            let packed = wasapi_format::pack(&format, &packet)?;
            render.write_to_device(written, &packed, None)?;
            timeline.submitted(written as u64);
            control
                .frames_written
                .fetch_add(packet_frames as u64, Ordering::Relaxed);
            packet_frames = 0;
            publish(false, meter.clone(), timeline.continuity_lost)?;
            if !started {
                if !control.start.wait(&control)?{break;}
                client.start_stream()?;
                started = true;
            }
        }
        Ok(())
    })();
    if scheduled&&!started{let _=client.reset_stream();}
    if started {
        let _ = client.stop_stream();
        let _ = client.reset_stream();
    }
    result
}
