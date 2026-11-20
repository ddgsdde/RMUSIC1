use crate::{auth::Auth, error::{ApiError, ApiResult}};
use axum::Json;
use rmusic_audio::dsp::{DspConfig, DspProcessor, EqBand, EqFilter};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Import {
    text: String,
}

#[derive(Deserialize)]
pub struct Preset {
    #[serde(default, alias = "preamp")]
    preamp_db: f64,
    #[serde(alias = "filters")]
    bands: Vec<EqBand>,
}

pub async fn import(auth: Auth, Json(input): Json<Import>) -> ApiResult<Json<PresetResponse>> {
    auth.require("control")?;
    let preset = parse(&input.text).map_err(|error| ApiError::bad(error.to_string()))?;
    Ok(Json(PresetResponse { preamp_db: preset.preamp_db, bands: preset.bands }))
}

#[derive(serde::Serialize)]
pub struct PresetResponse {
    preamp_db: f64,
    bands: Vec<EqBand>,
}

fn parse(text: &str) -> anyhow::Result<Preset> {
    use anyhow::{bail, ensure, Context};
    let text = text.trim_start_matches('\u{feff}').trim();
    ensure!(!text.is_empty() && text.len() <= 64 * 1024, "均衡预设为空或超过 64 KiB");
    let preset = if text.starts_with('{') {
        serde_json::from_str(text).context("JSON 预设无效：需要 bands 列表")?
    } else if text.starts_with('[') {
        Preset { preamp_db: 0., bands: serde_json::from_str(text).context("JSON 均衡频段无效")? }
    } else if text.lines().next().is_some_and(|line| line.trim().eq_ignore_ascii_case("type,frequency,gain_db,q")) {
        let bands = text.lines().skip(1).filter(|line| !line.trim().is_empty()).map(|line| {
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            ensure!(fields.len() == 4, "CSV 均衡频段需要类型、频率、增益和 Q");
            Ok(EqBand {
                kind: serde_json::from_value(serde_json::Value::String(fields[0].to_ascii_lowercase())).context("CSV 滤波器类型无效")?,
                enabled: true, frequency: fields[1].parse()?, gain_db: fields[2].parse()?, q: fields[3].parse()?,
                slope_db_oct: None,
            })
        }).collect::<anyhow::Result<Vec<_>>>()?;
        Preset { preamp_db: 0., bands }
    } else {
        let mut preset = Preset { preamp_db: 0., bands: Vec::new() };
        for (index, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or_default().trim();
            let (label, value) = line.split_once(':').unwrap_or((line, ""));
            if label.eq_ignore_ascii_case("preamp") {
                preset.preamp_db = value.split_whitespace().next().context("缺少前级增益")?.parse()?;
            } else if label.split_whitespace().next().is_some_and(|word| word.eq_ignore_ascii_case("filter")) {
                if value.split_whitespace().map(str::to_ascii_uppercase).eq(["OFF".to_owned(), "NONE".to_owned()]) {
                    continue;
                }
                preset.bands.push(parse_filter(value).with_context(|| format!("第 {} 行：{}", index + 1, line))?);
            } else if ["graphiceq", "include", "channel", "device"].iter().any(|directive| label.eq_ignore_ascii_case(directive)) {
                bail!("请导出独立的参数均衡预设，当前文件使用了 {label}");
            }
        }
        preset
    };
    ensure!(!preset.bands.is_empty(), "未找到参数均衡频段");
    DspProcessor::new(DspConfig {
        preamp_db: preset.preamp_db,
        bands: preset.bands.clone(),
        ..Default::default()
    }, 48000, 2, None).context("均衡预设超出频段、前级增益、频率、增益或 Q 的支持范围")?;
    Ok(preset)
}

fn parse_filter(line: &str) -> anyhow::Result<EqBand> {
    use anyhow::{bail, Context};
    let values: Vec<_> = line.split_whitespace().collect();
    let enabled = match values.first().map(|word| word.to_ascii_uppercase()).as_deref() {
        Some("ON") => true,
        Some("OFF") => false,
        _ => bail!("频段缺少 ON/OFF"),
    };
    let kind = match values.get(1).map(|word| word.to_ascii_uppercase()).as_deref() {
        Some("PK" | "PEQ") => EqFilter::Peak,
        Some("LS" | "LSC") => EqFilter::LowShelf,
        Some("HS" | "HSC") => EqFilter::HighShelf,
        Some("LP" | "LPQ") => EqFilter::LowPass,
        Some("HP" | "HPQ") => EqFilter::HighPass,
        Some("BP") => EqFilter::BandPass,
        Some("NO" | "NOTCH") => EqFilter::Notch,
        Some("AP") => EqFilter::AllPass,
        _ => bail!("不支持此滤波器类型"),
    };
    let parameter = |name: &str| -> anyhow::Result<Option<f64>> {
        if let Some(index) = values.iter().position(|word| word.eq_ignore_ascii_case(name)) {
            Ok(Some(values.get(index + 1).with_context(|| format!("缺少 {name} 数值"))?.parse()?))
        } else {
            Ok(None)
        }
    };
    Ok(EqBand {
        enabled,
        kind,
        frequency: parameter("Fc")?.context("缺少截止频率 Fc")?,
        gain_db: parameter("Gain")?.unwrap_or(0.),
        q: parameter("Q")?.unwrap_or(std::f64::consts::FRAC_1_SQRT_2),
        slope_db_oct: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_parametric_text_and_preserves_disabled_bands() {
        let preset = parse("Equaliser: Generic\nPreamp: -4.5 dB\nFilter 1: ON PK Fc 110 Hz Gain -3.2 dB Q 1.3\nFilter 2: OFF HS Fc 9500 Hz Gain 2.0 dB\nFilter 3: OFF None").unwrap();
        assert_eq!(preset.preamp_db, -4.5);
        assert_eq!(preset.bands[0].gain_db, -3.2);
        assert_eq!(preset.bands[1].kind, EqFilter::HighShelf);
        assert!(!preset.bands[1].enabled);
    }

    #[test]
    fn imports_json_and_rejects_unrepresentable_filters() {
        assert_eq!(parse(r#"[{"frequency":28000,"gain_db":0,"q":2,"kind":"notch"}]"#).unwrap().bands[0].kind, EqFilter::Notch);
        assert_eq!(parse("type,frequency,gain_db,q\nlow_shelf,120,-6,0.7").unwrap().bands[0].kind, EqFilter::LowShelf);
        assert!(parse("Filter: ON XX Fc 120 Hz").is_err());
        assert!(parse("GraphicEQ: 20 2; 100 -4").is_err());
    }
}
