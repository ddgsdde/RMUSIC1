#[cfg(any(feature="adaptive-sync",feature="format-conversion"))]
pub mod adaptive;
#[cfg(feature="format-conversion")]
pub mod resample;
pub mod decoder;
pub mod file;
pub mod dsd;
pub mod convolution;
pub mod impulse;
pub mod dsp;
pub mod fade;
pub mod output;
pub mod probe;

/// Listening/recovery clock limit, including eight-bit frames of native DSD512.
/// PCM device negotiation retains its independent 768 kHz limit.
pub const MAX_CLOCK_RATE:u32=3_072_000;

/// Prefer an explicitly configured tool, then the portable distribution beside the executable.
pub fn tool_binary(name: &str) -> std::ffi::OsString {
    let variable = match name {
        "ffmpeg" => "RMUSIC_FFMPEG",
        "ffprobe" => "RMUSIC_FFPROBE",
        _ => return name.into(),
    };
    if let Some(path) = std::env::var_os(variable) {
        return path;
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            let filename = if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            };
            let path = directory.join("tools").join(filename);
            if path.is_file() {
                return path.into_os_string();
            }
        }
    }
    name.into()
}

pub use dsp::{DspConfig, DspProcessor, EqBand, Meter};

pub mod visualization;
