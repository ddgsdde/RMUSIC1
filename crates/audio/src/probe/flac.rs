//! Read the lossless programme boundary from STREAMINFO, never a rounded duration tag.
use serde_json::Value;

pub(super) fn samples(stream:&Value,rate:u32,channels:u16,bits:u16)->Option<u64> {
    if stream["codec_name"]!="flac"||stream["extradata_size"].as_u64()!=Some(34){return None;}
    let dump=stream["extradata"].as_str()?;
    if dump.len()>1024{return None;}
    let mut bytes=Vec::with_capacity(34);
    for line in dump.lines().filter(|line|!line.is_empty()) {
        let (_,data)=line.split_once(':')?;
        let hex=data.trim_start().split("  ").next()?;
        for group in hex.split_whitespace() {
            if group.len()%2!=0||!group.is_ascii(){return None;}
            for pair in group.as_bytes().chunks_exact(2) {
                bytes.push(u8::from_str_radix(std::str::from_utf8(pair).ok()?,16).ok()?);
                if bytes.len()>34{return None;}
            }
        }
    }
    if bytes.len()!=34{return None;}
    let packed=u64::from_be_bytes(bytes[10..18].try_into().ok()?);
    let count=packed&((1u64<<36)-1);
    (packed>>44==rate as u64&&((packed>>41)&7)+1==channels as u64&&((packed>>36)&31)+1==bits as u64&&count>0).then_some(count)
}
