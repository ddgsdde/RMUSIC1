use anyhow::{bail, Context, Result};
use std::{
    io::{BufReader, Read},
    process::{Child, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex},
};

fn is_remote_proxy(input:&str)->bool {
    url::Url::parse(input).ok().is_some_and(|u|u.scheme()=="http"&&u.host_str()==Some("127.0.0.1")&&u.path().starts_with("/audio/")&&u.query_pairs().any(|(k,v)|k=="roaming"&&v=="1"))
}

fn is_converted_proxy(input:&str)->bool {
    url::Url::parse(input).ok().is_some_and(|u|u.scheme()=="http"&&u.host_str()==Some("127.0.0.1")&&u.path().starts_with("/audio/")&&u.query_pairs().any(|(k,v)|(k=="roaming"&&v=="1")||(k=="conversion"&&v=="core")))
}

fn core_pcm_format(input: &str) -> Option<(&'static str, u32, u16)> {
    let url = url::Url::parse(input).ok()?;
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1")
        || !url.path().starts_with("/audio/")
        || !url.query_pairs().any(|(key, value)| key == "core_pcm" && matches!(value.as_ref(), "render" | "compatibility"))
        || !url.query_pairs().any(|(key, value)| key == "conversion" && value == "core")
    {
        return None;
    }
    let get = |name| url.query_pairs().find(|(key, _)| key == name).map(|(_, value)| value);
    let format = match get("bits")?.as_ref() {
        "16" => "s16le",
        "24" => "s24le",
        "32" => "s32le",
        _ => return None,
    };
    Some((format, get("rate")?.parse().ok()?, get("channels")?.parse().ok()?))
}

#[derive(Debug, Clone, Copy)]
pub enum PcmEncoding {
    Signed32,
    Float64,
}

pub struct FfmpegDecoder {
    child: Arc<Mutex<Child>>,
    reader: BufReader<ChildStdout>,
    remaining: Option<u64>,
    diagnostic: Arc<Mutex<Vec<u8>>>,
    diagnostic_reader: Option<std::thread::JoinHandle<()>>,
}
/// Byte transport only: FFmpeg opens the URL/file as data and copies it without an audio decoder.
pub struct FfmpegBytes(FfmpegDecoder);
impl FfmpegBytes {
    pub fn open(input:&str,offset:u64)->Result<Self> {
        let mut command=Command::new(crate::tool_binary("ffmpeg"));
        // HTTP's initial Range avoids a best-effort demuxer skip silently staying at byte zero.
        let scheme=input.split(':').next().unwrap_or("");
        let offset_option=if scheme.eq_ignore_ascii_case("http")||scheme.eq_ignore_ascii_case("https"){"-offset"}else{"-skip_initial_bytes"};
        command.args(["-nostdin","-hide_banner","-loglevel","error","-xerror","-rw_timeout","15000000",
            offset_option,&offset.to_string(),"-f","data","-i",input,"-map","0:0","-c","copy","-f","data","pipe:1"]);
        Ok(Self(FfmpegDecoder::spawn(command)?))
    }
    pub fn abort_handle(&self)->DecoderAbort {self.0.abort_handle()}
}
impl Read for FfmpegBytes {
    fn read(&mut self,output:&mut[u8])->std::io::Result<usize> {self.0.reader.read(output)}
}
#[derive(Clone)]
pub struct DecoderAbort {
    child:Option<Arc<Mutex<Child>>>,
    #[cfg(target_os="android")]
    native:Option<Arc<dyn Fn()+Send+Sync>>,
}
#[derive(Debug)]
pub struct DecodeFailure(pub String);
impl std::fmt::Display for DecodeFailure {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str(&self.0)}}
impl std::error::Error for DecodeFailure {}
impl DecoderAbort {
    pub fn abort(&self) {
        if let Some(child)=&self.child {if let Ok(mut child)=child.lock(){let _=child.kill();}}
        #[cfg(target_os="android")] if let Some(native)=&self.native {native();}
    }
    #[cfg(target_os="android")]
    pub fn native(cancel:impl Fn()+Send+Sync+'static)->Self {Self {child:None,native:Some(Arc::new(cancel))}}
}
impl FfmpegDecoder {
    pub fn abort_handle(&self) -> DecoderAbort { DecoderAbort {child:Some(self.child.clone()),#[cfg(target_os="android")] native:None} }
    /// FFmpeg is a separate bounded streaming process. No whole-song allocation or shell execution.
    pub fn open(
        input: &str,
        position: f64,
        rate: u32,
        channels: u16,
        encoding: PcmEncoding,
    ) -> Result<Self> {
        Self::open_flac(input,position,rate,channels,encoding,None)
    }
    /// The optional extent must come from native STREAMINFO, not catalogue duration.
    pub fn open_flac(input:&str,position:f64,rate:u32,channels:u16,encoding:PcmEncoding,extent:Option<(u64,u32)>)->Result<Self> {
        if !position.is_finite() || position < 0. || rate < 8000 || channels == 0 {
            bail!("invalid decoder format")
        }
        let exe = crate::tool_binary("ffmpeg");
        let mut command = Command::new(exe);
        command.args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            // FFmpeg can otherwise return success after a partial HTTP read / timeout.
            // A decoder failure must never be interpreted as a natural queue boundary.
            "-rw_timeout",
            if is_remote_proxy(input){"45000000"}else{"15000000"},
            "-threads",
            "2",
            "-filter_threads",
            "1",
        ]);
        if is_converted_proxy(input) {
            // These are fresh, non-seekable HTTP streams. In FFmpeg's default HTTP
            // mode, a second read after the last chunk sees a closed handle as EIO.
            // Persistent mode tracks chunkend and reports EOF without relaxing errors.
            command.args(["-seekable","0","-multiple_requests","1"]);
        }
        if extent.is_some(){
            // FLAC demuxer seeking/post-roll can encounter bytes after the declared
            // programme. Reject corrupt audio frames and require every STREAMINFO sample
            // instead of making that ancillary seek error abort valid PCM output.
            command.args(["-err_detect","crccheck+explode"]);
        }else{command.arg("-xerror");}
        // FFmpeg time seeks use microseconds, which lose a sample at high PCM rates.
        // Seek whole seconds, then trim exact output-rate samples after resampling.
        let seek_filter=if position>0. {
            let frame=(position*rate as f64).round();let seconds=(frame/rate as f64).floor();
            if seconds>0. {command.args(["-ss",&seconds.to_string()]);}
            Some(format!("aresample={rate}:filter_size=64:phase_shift=10:exact_rational=1,atrim=start_sample={}",(frame-seconds*rate as f64) as u64))
        }else{Some(format!("aresample={rate}:filter_size=64:phase_shift=10:exact_rational=1"))};
        let format = match encoding {
            PcmEncoding::Signed32 => "s32le",
            PcmEncoding::Float64 => "f64le",
        };
        if let Some((format, rate, channels)) = core_pcm_format(input) {
            // The confirmed PCM format is carried by the media proxy. A streamed WAV
            // cannot signal its last partial packet accurately with an unknown RIFF size.
            command.args(["-f", format, "-ar", &rate.to_string(), "-ac", &channels.to_string(),
                "-probesize", "32", "-analyzeduration", "1"]);
        }
        command
            .args([
                "-i",
                input,
                "-map",
                "0:a:0",
                "-vn",
                "-sn",
                "-dn",
                "-ac",
                &channels.to_string(),
                "-ar",
                &rate.to_string(),
            ]);
        if let Some(filter)=seek_filter {command.args(["-af",&filter]);}
        command.args(["-f",format,"pipe:1"]);
        let mut decoder=Self::spawn(command)?;
        if let Some((samples,source_rate))=extent {
            anyhow::ensure!(source_rate>=8000&&samples>0,"invalid native FLAC extent");
            let end=(samples as u128*rate as u128).div_ceil(source_rate as u128);
            let start=(position*rate as f64).round() as u128;
            decoder.remaining=Some(u64::try_from(end.saturating_sub(start))?);
        }
        Ok(decoder)
    }
    fn spawn(mut command:Command)->Result<Self> {
        command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .context("FFmpeg unavailable; install FFmpeg or set RMUSIC_FFMPEG")?;
        let reader = BufReader::new(child.stdout.take().context("missing decoder stdout")?);
        let mut stderr=child.stderr.take().context("missing decoder diagnostics")?;
        let diagnostic=Arc::new(Mutex::new(Vec::new()));let saved=diagnostic.clone();
        let diagnostic_reader=match std::thread::Builder::new().name("audio-diagnostics".into()).spawn(move||{
            let mut chunk=[0u8;2048];
            loop {match stderr.read(&mut chunk) {
                Ok(0)=>break,
                Ok(n)=>{if let Ok(mut tail)=saved.lock(){let discard=(tail.len()+n).saturating_sub(8192);if discard>0{tail.drain(..discard);}tail.extend_from_slice(&chunk[..n]);}},
                Err(e) if e.kind()==std::io::ErrorKind::Interrupted=>continue,
                Err(_)=>break,
            }}
        }){Ok(reader)=>reader,Err(error)=>{let _=child.kill();let _=child.wait();return Err(error.into());}};
        Ok(Self { child: Arc::new(Mutex::new(child)), reader, remaining:None, diagnostic, diagnostic_reader:Some(diagnostic_reader) })
    }
    pub fn read_frames(&mut self, output: &mut [u8], frame_bytes: usize) -> Result<usize> {
        if frame_bytes == 0 || output.len() % frame_bytes != 0 {
            bail!("invalid output buffer")
        }
        if self.remaining==Some(0){return Ok(0);}
        let capacity=self.remaining.map_or(output.len(),|left|output.len().min(left.min(usize::MAX as u64/frame_bytes as u64) as usize*frame_bytes));
        let mut filled = 0;
        let mut eof = false;
        while filled < capacity {
            match self.reader.read(&mut output[filled..capacity]) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(DecodeFailure(e.to_string()).into()),
            }
        }
        if filled % frame_bytes != 0 {
            return Err(DecodeFailure("decoder returned truncated PCM frame".into()).into());
        }
        if eof {
            let status=self.child.lock().map_err(|_|anyhow::anyhow!("decoder lock poisoned"))?.wait()?;
            if !status.success() {
                if let Some(reader)=self.diagnostic_reader.take(){let _=reader.join();}
                let message=self.diagnostic.lock().map(|value|decode_failure_message(&String::from_utf8_lossy(&value))).unwrap_or("音源读取或解码中断，请重试当前歌曲");
                return Err(DecodeFailure(message.into()).into());
            }
        }
        if let Some(left)=&mut self.remaining {
            let frames=(filled/frame_bytes) as u64;
            if eof&&frames<*left {return Err(DecodeFailure("FLAC 数据提前结束，尚未达到 STREAMINFO 声明的采样数".into()).into());}
            *left-=frames;
            // All declared samples passed strict frame decoding. Post-programme junk must
            // not discard the final partial PCM packet or be mistaken for a corrupt frame.
            if *left==0 {if let Ok(mut child)=self.child.lock(){let _=child.kill();let _=child.wait();}}
        }
        Ok(filled / frame_bytes)
    }
}
impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader)=self.diagnostic_reader.take(){let _=reader.join();}
    }
}

// Never expose raw FFmpeg stderr: it can contain a signed media URL or local path.
fn decode_failure_message(diagnostic:&str)->&'static str {
    let text=diagnostic.to_ascii_lowercase();
    if ["401 unauthorized","403 forbidden","server returned 401","server returned 403"].iter().any(|value|text.contains(value)) {
        "音源访问被拒绝，请刷新播放地址或检查平台登录后重试"
    }else if ["timed out","timeout","connection reset","connection refused","input/output error","stream ends prematurely"].iter().any(|value|text.contains(value)) {
        "音频流传输中断，请检查 Core 连接后重试当前歌曲"
    }else if ["invalid sync code","invalid frame header","crc mismatch","decoding error","decode_frame() failed"].iter().any(|value|text.contains(value)) {
        "音频数据校验失败，请重新获取音源后重试（不会自动跳过损坏数据）"
    }else {"音源读取或解码中断，请重试当前歌曲"}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn decoder_diagnostics_classify_without_disclosing_media_authority() {
        for (input,expected) in [
            ("https://private.test/song?ticket=secret: 403 Forbidden", "访问被拒绝"),
            ("https://private.test/song?ticket=secret: Connection reset by peer", "传输中断"),
            ("/private/music.flac: invalid sync code; invalid frame header", "数据校验失败"),
        ] {let message=decode_failure_message(input);assert!(message.contains(expected));assert!(!message.contains("secret")&&!message.contains("private"));}
    }
    #[test]
    fn failed_http_decoder_captures_and_drains_private_diagnostics() {
        use std::net::TcpListener;
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
        let server=std::thread::spawn(move||{let(mut socket,_)=listener.accept().unwrap();let mut buffer=[0;4096];let _=socket.read(&mut buffer);socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();});
        let mut decoder=FfmpegDecoder::open(&format!("http://{address}/audio/private?ticket=do-not-expose"),0.,48000,2,PcmEncoding::Signed32).unwrap();
        let error=decoder.read_frames(&mut[0;4096],8).unwrap_err().to_string();
        assert!(error.contains("访问被拒绝"),"{error}");assert!(!error.contains("do-not-expose"));
        assert!(decoder.diagnostic.lock().unwrap().len()<=8192);server.join().unwrap();
    }
    #[test]
    fn cancellation_interrupts_a_blocked_network_decoder() {
        use std::{net::TcpListener,sync::mpsc,time::Duration};
        let server=TcpListener::bind("127.0.0.1:0").unwrap();
        let (accepted,connected)=mpsc::channel();let (release,wait)=mpsc::channel();
        let address=server.local_addr().unwrap();
        let server_thread=std::thread::spawn(move||{let (socket,_)=server.accept().unwrap();accepted.send(()).unwrap();let _=wait.recv_timeout(Duration::from_secs(10));drop(socket);});
        let mut decoder=FfmpegDecoder::open(&format!("http://{address}/delayed.wav"),0.,48000,2,PcmEncoding::Signed32).unwrap();
        let abort=decoder.abort_handle();let (done,finished)=mpsc::channel();
        let worker=std::thread::spawn(move||{let result=decoder.read_frames(&mut [0;1024],8);done.send(result.is_err()).unwrap();});
        connected.recv_timeout(Duration::from_secs(5)).unwrap();
        abort.abort();
        let interrupted=finished.recv_timeout(Duration::from_secs(1));
        let _=release.send(());server_thread.join().unwrap();worker.join().unwrap();
        assert_eq!(interrupted.unwrap(),true);
    }
    fn wav(bits: u16, samples: &[i32]) -> tempfile::NamedTempFile {
        let mut file = tempfile::Builder::new().suffix(".wav").tempfile().unwrap();
        let width = bits / 8;
        let length = samples.len() as u32 * width as u32;
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + length).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16u32.to_le_bytes()).unwrap();
        file.write_all(&1u16.to_le_bytes()).unwrap();
        file.write_all(&1u16.to_le_bytes()).unwrap();
        file.write_all(&48000u32.to_le_bytes()).unwrap();
        file.write_all(&(48000 * width as u32).to_le_bytes())
            .unwrap();
        file.write_all(&width.to_le_bytes()).unwrap();
        file.write_all(&bits.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&length.to_le_bytes()).unwrap();
        for sample in samples {
            file.write_all(&sample.to_le_bytes()[..width as usize])
                .unwrap();
        }
        file.flush().unwrap();
        file
    }
    #[test]
    fn ffmpeg_pcm16_and_pcm24_preserve_every_integer_sample() {
        for (bits, samples) in [
            (16, vec![-32768, -1000, -1, 0, 1, 1000, 32767]),
            (24, vec![-8388608, -1234567, -1, 0, 1, 1234567, 8388607]),
        ] {
            let file = wav(bits, &samples);
            let mut decoder = FfmpegDecoder::open(
                file.path().to_str().unwrap(),
                0.,
                48000,
                1,
                PcmEncoding::Signed32,
            )
            .unwrap();
            let mut bytes = [0u8; 1024];
            let frames = decoder.read_frames(&mut bytes, 4).unwrap();
            let decoded = bytes[..frames * 4]
                .chunks_exact(4)
                .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(
                decoded,
                samples
                    .iter()
                    .map(|sample| sample << (32 - bits))
                    .collect::<Vec<_>>()
            );
            assert_eq!(decoder.read_frames(&mut bytes, 4).unwrap(), 0);
        }
    }
    #[test]
    fn ffmpeg_pcm_seek_retains_exact_sample_at_48k_and_768k_for_integer_and_float_output() {
        use std::io::{Seek,SeekFrom};
        for rate in [48000u32,768000] {
            let samples:Vec<i32>=(0..rate+16384).map(|i|i.wrapping_mul(15485863) as i32).collect();
            let mut file=wav(32,&samples);file.as_file_mut().seek(SeekFrom::Start(24)).unwrap();file.write_all(&rate.to_le_bytes()).unwrap();file.write_all(&(rate*4).to_le_bytes()).unwrap();file.flush().unwrap();
            for encoding in [PcmEncoding::Signed32,PcmEncoding::Float64] {for offset in [8189usize,rate as usize+8189] {
                let width=match encoding {PcmEncoding::Signed32=>4,PcmEncoding::Float64=>8};let mut decoder=FfmpegDecoder::open(file.path().to_str().unwrap(),offset as f64/rate as f64,rate,1,encoding).unwrap();let mut actual=Vec::new();let mut buffer=[0u8;8192];
                loop {let n=decoder.read_frames(&mut buffer,width).unwrap();if n==0{break;}actual.extend_from_slice(&buffer[..n*width]);}
                let expected:Vec<u8>=samples[offset..].iter().flat_map(|&sample|match encoding {PcmEncoding::Signed32=>sample.to_le_bytes().to_vec(),PcmEncoding::Float64=>(sample as f64/2147483648.).to_le_bytes().to_vec()}).collect();
                assert_eq!(actual.len(),expected.len(),"rate={rate} offset={offset} encoding={encoding:?}");assert!(actual==expected,"sample bytes differ: rate={rate} offset={offset} encoding={encoding:?}");
            }}
        }
    }
    #[test]
    fn ffmpeg_pcm_resampled_seek_trims_at_output_rate() {
        let samples:Vec<i32>=(0..96000u32).map(|i|i.wrapping_mul(15485863) as i32).collect();let file=wav(32,&samples);
        let decode=|position|{let mut decoder=FfmpegDecoder::open(file.path().to_str().unwrap(),position,96000,1,PcmEncoding::Float64).unwrap();let mut bytes=Vec::new();let mut buffer=[0u8;8192];loop {let n=decoder.read_frames(&mut buffer,8).unwrap();if n==0{break;}bytes.extend_from_slice(&buffer[..n*8]);}bytes};
        let reference=decode(0.);for frame in [8189usize,96000+8189] {let actual=decode(frame as f64/96000.);assert_eq!(actual.len(),reference.len()-frame*8);assert!(actual==reference[frame*8..],"resampled source tail differs at output frame {frame}");}
    }
    #[test]
    fn http_read_error_after_a_seek_is_not_a_successful_end_of_song() {
        use std::{io::{Read, Write}, net::TcpListener};
        let source = wav(16, &vec![123; 48000 * 4]);
        let mut bytes = std::fs::read(source.path()).unwrap();
        let length = bytes.len();
        bytes.truncate(44 + 48000 * 2); // Two seconds delivered from a four-second source.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096]; socket.read(&mut request).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").unwrap();
            socket.write_all(&bytes).unwrap();
        });
        let mut decoder = FfmpegDecoder::open(&format!("http://{address}/source.wav"),0.5,48000,1,PcmEncoding::Signed32).unwrap();
        let mut frames = 0;
        let failed = loop {
            match decoder.read_frames(&mut [0; 4096],4) {
                Ok(0) => break false,
                Ok(count) => frames += count,
                Err(error) => { assert!(error.is::<DecodeFailure>()); break true; }
            }
        };
        server.join().unwrap();
        assert!(frames > 0 && frames < 48000 * 3);
        assert!(failed, "FFmpeg must propagate a partial HTTP read instead of advancing the queue");
    }
    #[test]
    fn flac_streaminfo_boundary_preserves_the_complete_tail_and_rejects_real_damage() {
        use std::io::Write;
        let samples:Vec<i32>=(0..100003i32).map(|i|((i*997)%60000)-30000).collect();
        let source=wav(16,&samples);let encoded=tempfile::Builder::new().suffix(".flac").tempfile().unwrap();
        assert!(Command::new(crate::tool_binary("ffmpeg")).args(["-v","error","-i"]).arg(source.path()).args(["-c:a","flac","-y"]).arg(encoded.path()).status().unwrap().success());
        let bytes=std::fs::read(encoded.path()).unwrap();let mut audio=4;
        loop {let last=bytes[audio]&128!=0;let length=u32::from_be_bytes([0,bytes[audio+1],bytes[audio+2],bytes[audio+3]]) as usize;audio+=4+length;if last{break;}}
        let mut trailing=tempfile::Builder::new().suffix(".flac").tempfile().unwrap();trailing.write_all(&bytes).unwrap();trailing.write_all(&[0;2048]).unwrap();trailing.flush().unwrap();
        let metadata=crate::probe::probe(trailing.path()).unwrap();assert_eq!(metadata.flac_samples,Some(samples.len() as u64));
        let decode=|file:&std::path::Path,rate,offset,extent|->Result<Vec<u8>> {
            let mut decoder=FfmpegDecoder::open_flac(file.to_str().unwrap(),offset,rate,1,PcmEncoding::Signed32,extent)?;
            let mut data=Vec::new();let mut buffer=[0;4096];
            loop {let n=decoder.read_frames(&mut buffer,4)?;if n==0{assert_eq!(decoder.read_frames(&mut buffer,4)?,0);break;}data.extend_from_slice(&buffer[..n*4]);}Ok(data)
        };
        assert!(decode(trailing.path(),48000,0.,None).unwrap_err().is::<DecodeFailure>(),"baseline must expose the post-programme error");
        for rate in [44100,48000,96000] {
            let reference=decode(encoded.path(),rate,0.,None).unwrap();
            for frame in [0,rate as usize+819,reference.len()/4] {
                let decoded=decode(trailing.path(),rate,frame as f64/rate as f64,Some((samples.len() as u64,48000))).unwrap();
                assert_eq!(decoded.len(),reference.len()-frame*4,"rate={rate} frame={frame}");assert!(decoded==reference[frame*4..],"lossless tail bytes differ");
            }
        }
        for corrupted in [false,true] {
            let mut damaged=bytes.clone();
            if corrupted {damaged[audio+50]^=0x40;}else{damaged.truncate(damaged.len()-100);}
            let mut file=tempfile::Builder::new().suffix(".flac").tempfile().unwrap();file.write_all(&damaged).unwrap();file.flush().unwrap();
            assert!(decode(file.path(),48000,0.,Some((samples.len() as u64,48000))).unwrap_err().is::<DecodeFailure>(),"real truncated/corrupt audio must not become EOF");
        }
    }
    #[test]
    fn ffmpeg_missing_input_reports_error() {
        let file = tempfile::tempdir()
            .unwrap()
            .path()
            .join("does-not-exist.flac");
        let mut decoder =
            FfmpegDecoder::open(file.to_str().unwrap(), 0., 48000, 2, PcmEncoding::Signed32)
                .unwrap();
        assert!(decoder.read_frames(&mut [0; 1024], 8).is_err());
    }
}
