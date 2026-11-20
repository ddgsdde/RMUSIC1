use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Stdio},
};
#[path="probe/flac.rs"]
mod flac;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioMetadata {
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub flac_samples: Option<u64>,
    #[serde(default)]
    pub duration: Option<f64>,
    pub format: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_depth: u16,
    pub bitrate: u32,
    pub quality_rank: u32,
    pub source_integer_lossless: bool,
}
impl AudioMetadata {
    pub fn is_dsd(&self)->bool {matches!(self.format.as_str(),"dsf"|"dff")}
    /// PCM target for DSD decoding: retain its rate family within the native PCM ceiling.
    pub fn dsd_pcm_rate(&self)->u32 {
        let mut rate=self.sample_rate/16;
        while rate>768000 {rate/=2;}
        rate
    }
    /// Device-frame timebase: one DoP carrier frame transports sixteen DSD bits.
    pub fn output_rate(&self)->u32 {if self.is_dsd(){self.sample_rate/16}else{self.sample_rate}}
    pub fn output_rate_for(&self,mode:crate::output::DsdOutput)->u32 {
        if self.is_dsd()&&mode==crate::output::DsdOutput::Native {self.sample_rate/8}else{self.output_rate()}
    }
}
pub fn quality_rank(format: &str, rate: u32, bits: u16, bitrate: u32) -> u32 {
    if matches!(format, "flac" | "alac" | "wav" | "aiff") && rate > 0 && bits > 0 {
        1_000_000_000u32
            .saturating_add(bits as u32 * 1_000_000)
            .saturating_add(rate)
    } else if matches!(format, "mp3" | "aac" | "m4a" | "ogg" | "opus") {
        bitrate
    } else {
        0
    }
}
/// Only verified PCM metadata may bypass a second network probe. DSD keeps its exact parser.
pub fn verified_pcm(value:&Value)->Option<AudioMetadata>{
 if value["verified"]!=true{return None;}
 let format=value["format"].as_str()?;
 if !matches!(format,"flac"|"alac"){return None;}
 let rate=u32::try_from(value["sample_rate"].as_u64()?).ok()?;let bits=u16::try_from(value["bit_depth"].as_u64()?).ok()?;let channels=u16::try_from(value["channels"].as_u64()?).ok()?;
 if !(8000..=768000).contains(&rate)||!matches!(bits,8|16|20|24|32)||!(1..=8).contains(&channels){return None;}
 Some(AudioMetadata{flac_samples:None,duration:value["duration"].as_f64().filter(|v|v.is_finite()&&*v>0.),format:format.into(),sample_rate:rate,channels,bit_depth:bits,bitrate:value["bitrate"].as_u64().and_then(|v|v.try_into().ok()).unwrap_or(0),quality_rank:quality_rank(format,rate,bits,0),source_integer_lossless:true})
}
fn command(path: &Path) -> Command {
    let exe = crate::tool_binary("ffprobe");
    let mut command = Command::new(exe);
    command.args(["-v","error","-rw_timeout","15000000","-analyzeduration","1000000","-probesize","1048576","-select_streams","a:0","-show_data","-show_entries","stream=extradata,extradata_size,codec_name,sample_rate,channels,bits_per_raw_sample,bits_per_sample,bit_rate,duration:format=format_name,bit_rate,duration","-of","json"]).arg(path).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}
pub fn probe(path: &Path) -> Result<AudioMetadata> {
    #[cfg(target_os="android")] {return crate::output::android::backend()?.probe(path);}
    #[cfg(not(target_os="android"))] {
    let mut audio=parse(command(path).output().context("无法启动 FFprobe，请检查客户端配套音频工具是否完整")?)?;
    if audio.is_dsd()&&path.is_file() {
        let mut file=std::fs::File::open(path)?;
        let layout=crate::dsd::DsdLayout::read(&mut file)?;
        if file.metadata()?.len()<layout.data_end {bail!("truncated DSD programme data");}
        audio.duration=Some(layout.duration());
        audio.sample_rate=layout.sample_rate;audio.channels=layout.channels;
        audio.bitrate=layout.sample_rate*layout.channels as u32;
    }
    Ok(audio)
    }
}
/// Dropping this future terminates FFprobe; the entire probe also has a deadline.
pub async fn probe_async(path: &Path) -> Result<AudioMetadata> {
    #[cfg(target_os="android")] {
        let path=path.to_owned();return tokio::task::spawn_blocking(move||probe(&path)).await?;
    }
    #[cfg(not(target_os="android"))] {
    let mut command = tokio::process::Command::from(command(path));
    command.kill_on_drop(true);
    let child = command.spawn().context("无法启动 FFprobe，请检查客户端配套音频工具是否完整")?;
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
        .await.context("音频格式探测超时：20 秒内未收到有效音频头，请检查音源连接或限流状态")??;
    parse(output)
    }
}
#[derive(Debug)]
pub struct InvalidAudioHeader;

impl std::fmt::Display for InvalidAudioHeader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("音频格式校验失败：文件头无效、数据未完整返回或格式不受支持")
    }
}

impl std::error::Error for InvalidAudioHeader {}

fn parse(output: std::process::Output) -> Result<AudioMetadata> {
    if !output.status.success() {
        let diagnostic=String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        if diagnostic.contains("timed out")||diagnostic.contains("timeout") {bail!("音源传输超时：未收到有效音频数据，请检查 Core、音源连接或上游限流状态");}
        if diagnostic.contains("403")||diagnostic.contains("401") {bail!("音源访问被拒绝，请检查平台登录或重新获取播放地址");}
        if diagnostic.contains("http error")||diagnostic.contains("connection refused")||diagnostic.contains("error reading http") {bail!("音源网络请求失败，请检查 Core 与音源服务连接");}
    }
    if !output.status.success() || output.stdout.len() > 1024 * 1024 {
        return Err(InvalidAudioHeader.into());
    }
    let json: Value = serde_json::from_slice(&output.stdout)?;
    let stream = &json["streams"][0];
    let container = json["format"]["format_name"].as_str().unwrap_or("");
    let codec = stream["codec_name"].as_str().unwrap_or("");
    let format = match codec {
        "flac" => "flac",
        "alac" => "m4a",
        "mp3" => "mp3",
        "aac" => {
            if container.contains("mov") {
                "m4a"
            } else {
                "aac"
            }
        }
        "opus" => "opus",
        "vorbis" => "ogg",
        s if s.starts_with("pcm_") => {
            if container.contains("aiff") {
                "aiff"
            } else {
                "wav"
            }
        }
        "dsd_lsbf"|"dsd_msbf"|"dsd_lsbf_planar"|"dsd_msbf_planar"=>{
            if container=="dsf" {"dsf"}else if container=="iff" {"dff"}else{bail!("unsupported DSD container");}
        }
        _ => bail!("unsupported cached audio codec"),
    }
    .to_owned();
    let number = |value: &Value| {
        value
            .as_u64()
            .or_else(|| value.as_str()?.parse().ok())
            .unwrap_or(0)
    };
    let dsd=matches!(format.as_str(),"dsf"|"dff");
    let rate = number(&stream["sample_rate"]).checked_mul(if dsd{8}else{1}).filter(|rate|*rate<=u32::MAX as u64).context("invalid audio sample rate")? as u32;
    let bits = if dsd {1}else{
        number(&stream["bits_per_raw_sample"]).max(number(&stream["bits_per_sample"])) as u16
    };
    let channels = number(&stream["channels"]) as u16;
    let bitrate = number(&stream["bit_rate"]).max(number(&json["format"]["bit_rate"])) as u32;
    if rate == 0 || channels == 0 || channels > 32 {
        bail!("invalid cached audio metadata")
    }
    let quality = if codec == "alac" {
        quality_rank("alac", rate, bits, bitrate)
    } else {
        quality_rank(&format, rate, bits, bitrate)
    };
    let source_integer_lossless = (matches!(codec, "flac" | "alac")
        || codec.starts_with("pcm_s")
        || codec.starts_with("pcm_u"))
        && bits > 0
        && bits <= 32;
    Ok(AudioMetadata {
        flac_samples: flac::samples(stream,rate,channels,bits),
        duration: stream["duration"].as_str().or_else(||json["format"]["duration"].as_str()).and_then(|v|v.parse::<f64>().ok()).filter(|v|v.is_finite()&&*v>0.),
        format,
        sample_rate: rate,
        channels,
        bit_depth: bits,
        bitrate,
        quality_rank: quality,
        source_integer_lossless,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dsd_single_local_probe_preserves_bit_exact_duration_and_rejects_truncation() {
        use crate::dsd::tests::{samples,dsf,dff};
        use std::io::Write;
        for (extension,bytes,bits) in [(".dsf",dsf(&samples(514),4105,true),4105),(".dff",dff(&samples(515)),4120)] {
            let mut file=tempfile::Builder::new().suffix(extension).tempfile().unwrap();
            file.write_all(&bytes).unwrap();file.flush().unwrap();
            let audio=probe(file.path()).unwrap();
            assert_eq!(audio.duration,Some(bits as f64/2822400.));
            assert_eq!((audio.sample_rate,audio.output_rate(),audio.channels,audio.bit_depth),(2822400,176400,2,1));
            assert!(!audio.source_integer_lossless);
            file.as_file().set_len(bytes.len() as u64-1).unwrap();
            assert!(probe(file.path()).is_err(),"truncated programme data must not be verified for offline play");
        }
    }
    #[tokio::test]
    async fn probes_real_flac_and_does_not_mark_float_pcm_bitperfect() {
        let directory = tempfile::tempdir().unwrap();
        for (name, codec, sample_format, expected) in [
            ("sample.flac", "flac", "s32", true),
            ("float.wav", "pcm_f64le", "dbl", false),
        ] {
            let path = directory.path().join(name);
            assert!(Command::new(
                std::env::var_os("RMUSIC_FFMPEG").unwrap_or_else(|| "ffmpeg".into())
            )
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=96000:cl=stereo",
                "-t",
                "0.02",
                "-c:a",
                codec,
                "-sample_fmt",
                sample_format
            ])
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success());
            let audio = probe(&path).unwrap();
            let asynchronous=probe_async(&path).await.unwrap();
            assert_eq!(serde_json::to_value(&audio).unwrap(),serde_json::to_value(&asynchronous).unwrap());
            assert_eq!(audio.sample_rate, 96000);
            assert_eq!(audio.channels, 2);
            assert_eq!(audio.source_integer_lossless, expected);
        }
    }
    #[tokio::test]
    async fn cancelling_a_network_probe_closes_its_real_child_connection() {
        use tokio::io::AsyncReadExt;
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}/delayed.wav",listener.local_addr().unwrap());
        let probe=tokio::spawn(async move{probe_async(Path::new(&url)).await});
        let (mut socket,_)=tokio::time::timeout(std::time::Duration::from_secs(5),listener.accept()).await.unwrap().unwrap();
        probe.abort();assert!(probe.await.unwrap_err().is_cancelled());
        tokio::time::timeout(std::time::Duration::from_secs(1),async {
            let mut bytes=[0u8;4096];
            loop {match socket.read(&mut bytes).await {Ok(0)|Err(_)=>break,Ok(_)=>{}}}
        }).await.expect("cancelled FFprobe must close its socket without waiting for the read timeout");
    }
}

#[cfg(test)]mod verified_metadata_tests{use super::*;#[test]fn library_redesign_verified_pcm_skips_only_trusted_original_formats(){let mut value=serde_json::json!({"verified":true,"format":"flac","sample_rate":768000,"bit_depth":24,"channels":2,"duration":180.});assert_eq!(verified_pcm(&value).unwrap().sample_rate,768000);value["verified"]=serde_json::json!(false);assert!(verified_pcm(&value).is_none());value["verified"]=serde_json::json!(true);value["format"]=serde_json::json!("dsf");assert!(verified_pcm(&value).is_none());}}
