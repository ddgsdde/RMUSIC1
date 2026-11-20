use anyhow::{bail, Result};
use std::borrow::Cow;
use wasapi::{SampleType, WaveFormat};

/// Keep rate, channels and source precision; only the PCM container may change.
pub(super) fn exclusive(
    desired: &WaveFormat,
    source_bits: u16,
    supported: impl FnMut(&WaveFormat) -> std::result::Result<WaveFormat, wasapi::WasapiError>,
) -> Result<WaveFormat> {
    limited(desired,source_bits,None,supported)
}
pub(super) fn limited(desired:&WaveFormat,source_bits:u16,maximum:Option<u16>,mut supported:impl FnMut(&WaveFormat)->std::result::Result<WaveFormat,wasapi::WasapiError>)->Result<WaveFormat>{
    let float = matches!(desired.get_subformat()?, SampleType::Float);
    let minimum = if source_bits == 0 {32} else {source_bits.min(32)}.min(maximum.unwrap_or(32));
    let mut candidates = vec![desired.clone()];
    if !float {
        for (container, valid) in [(32, 24), (24, 24), (16, 16)] {
            if valid >= minimum&&valid<=maximum.unwrap_or(32) {
                candidates.push(WaveFormat::new(container, valid as usize, &SampleType::Int,
                    desired.get_samplespersec() as usize, desired.get_nchannels() as usize, None));
            }
        }
    }
    for candidate in candidates {
        if let Ok(actual) = supported(&candidate) {
            if actual.get_validbitspersample()<=maximum.unwrap_or(64)&&actual.get_samplespersec() == desired.get_samplespersec()
                && actual.get_nchannels() == desired.get_nchannels()
                && actual.get_bitspersample() == candidate.get_bitspersample()
                && actual.get_validbitspersample() == candidate.get_validbitspersample()
                && matches!(actual.get_subformat()?, SampleType::Float) == float
            {
                return Ok(actual);
            }
        }
    }
    bail!("设备不支持 {} Hz／{} 声道的{}独占输出（整数路径至少 {} 位有效精度），请更换输出设备或使用共享模式",
        desired.get_samplespersec(), desired.get_nchannels(), if float {"浮点"} else {"整数"}, minimum)
}

/// The decoder/continuous engine produces 32-bit samples. WAVEFORMATEXTENSIBLE
/// PCM uses left-aligned valid bits; packed 24-bit devices need three bytes/sample.
pub(super) fn pack<'a>(format: &WaveFormat, samples: &'a [u8]) -> Result<Cow<'a, [u8]>> {
    let channels = format.get_nchannels() as usize;
    if channels == 0 || samples.len() % (channels * 4) != 0 {bail!("incomplete WASAPI PCM frame");}
    let container = format.get_bitspersample();
    let valid = format.get_validbitspersample();
    if matches!(format.get_subformat()?, SampleType::Float) {
        if container != 32 || valid != 32 {bail!("unsupported WASAPI float format");}
        return Ok(Cow::Borrowed(samples));
    }
    if !matches!((container, valid), (32, 32) | (32, 24) | (24, 24) | (16, 16)) {
        bail!("unsupported WASAPI integer format");
    }
    if valid == 32 {return Ok(Cow::Borrowed(samples));}
    let width = container as usize / 8;
    let limit = 1_i64 << (valid - 1);
    let mut output = Vec::with_capacity(samples.len() / 4 * width);
    for bytes in samples.chunks_exact(4) {
        let sample = i32::from_le_bytes(bytes.try_into().unwrap());
        let value = (sample as f64 / (1_u64 << (32 - valid)) as f64).round()
            .clamp(-limit as f64, (limit - 1) as f64) as i64;
        let aligned = (value << (container - valid)) as i32;
        output.extend_from_slice(&aligned.to_le_bytes()[..width]);
    }
    Ok(Cow::Owned(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn format(container: usize, valid: usize, float: bool) -> WaveFormat {
        WaveFormat::new(container, valid, &if float {SampleType::Float} else {SampleType::Int}, 768000, 2, None)
    }
    #[test]
    fn negotiates_integer_containers_without_lowering_source_precision() {
        for (bits, container, valid, attempts) in [(24, 32, 24, 2), (24, 24, 24, 3), (16, 16, 16, 4)] {
            let mut queried = 0;
            let actual = exclusive(&format(32, 32, false), bits, |candidate| {
                queried += 1;
                if candidate.get_bitspersample() == container && candidate.get_validbitspersample() == valid {Ok(candidate.clone())}
                else {Err(wasapi::WasapiError::UnsupportedFormat)}
            }).unwrap();
            assert_eq!((actual.get_samplespersec(), actual.get_nchannels()), (768000, 2));
            assert_eq!((actual.get_bitspersample(), actual.get_validbitspersample(), queried), (container, valid, attempts));
        }
        for bits in [0, 32] {
            assert!(exclusive(&format(32, 32, false), bits, |_| Ok(format(32, 24, false))).is_err());
        }
        assert!(exclusive(&format(32, 32, true), 24, |_| Ok(format(32, 32, false))).is_err());
        assert!(exclusive(&format(32, 32, false), 16, |_| Ok(WaveFormat::new(32, 32, &SampleType::Int, 192000, 2, None))).is_err());
    }
    #[test]
    fn packs_signed_pcm_extrema_padding_rounding_and_stereo_order() {
        let samples = [i32::MIN, i32::MAX, 0, 256, -256, 0x12345600];
        let bytes = samples.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
        assert_eq!(pack(&format(24, 24, false), &bytes).unwrap().as_ref(),
            &[0,0,128, 255,255,127, 0,0,0, 1,0,0, 255,255,255, 86,52,18]);
        assert_eq!(pack(&format(32, 24, false), &bytes).unwrap().as_ref(),
            &[0,0,0,128, 0,255,255,127, 0,0,0,0, 0,1,0,0, 0,255,255,255, 0,86,52,18]);
        let samples = [i32::MIN, i32::MAX, 65536, -65536, 32768, -32768];
        let bytes = samples.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
        assert_eq!(pack(&format(16, 16, false), &bytes).unwrap().as_ref(), &[0,128, 255,127, 1,0, 255,255, 1,0, 255,255]);
        assert!(pack(&format(24, 24, false), &[0;4]).is_err());
    }
    #[test]
    fn unchanged_32bit_integer_and_float_paths_do_not_copy_or_change_bits() {
        let bytes = [0,0,0,128, 255,255,255,127];
        for float in [false, true] {
            let packed = pack(&format(32, 32, float), &bytes).unwrap();
            assert!(matches!(packed, Cow::Borrowed(_)));
            assert_eq!(packed.as_ref(), bytes);
        }
    }
    #[test]
    fn ffmpeg_768khz_integer_wav_packs_exactly_for_narrow_exclusive_devices() {
        use std::io::Write;
        for bits in [16_u16, 24] {
            let limit = 1_i32 << (bits - 1);
            let samples = [-limit, limit - 1, 1, -1, 0, 127];
            let width = bits as usize / 8;
            let raw = samples.iter().flat_map(|sample| sample.to_le_bytes()[..width].to_vec()).collect::<Vec<_>>();
            let mut file = tempfile::Builder::new().suffix(".wav").tempfile().unwrap();
            for bytes in [b"RIFF".as_slice(), &(36 + raw.len() as u32).to_le_bytes(), b"WAVEfmt ",
                &16_u32.to_le_bytes(), &1_u16.to_le_bytes(), &2_u16.to_le_bytes(), &768000_u32.to_le_bytes(),
                &(768000_u32 * 2 * width as u32).to_le_bytes(), &(2 * width as u16).to_le_bytes(),
                &bits.to_le_bytes(), b"data", &(raw.len() as u32).to_le_bytes(), &raw] {file.write_all(bytes).unwrap();}
            file.flush().unwrap();
            let mut decoder = crate::decoder::FfmpegDecoder::open(file.path().to_str().unwrap(), 0., 768000, 2,
                crate::decoder::PcmEncoding::Signed32).unwrap();
            let mut decoded = [0; 24];
            assert_eq!(decoder.read_frames(&mut decoded, 8).unwrap(), 3);
            assert_eq!(pack(&format(bits as usize, bits as usize, false), &decoded).unwrap().as_ref(), raw);
            if bits == 24 {
                assert_eq!(pack(&format(32, 24, false), &decoded).unwrap().as_ref(), decoded);
            }
        }
    }
}
