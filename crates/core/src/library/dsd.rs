//! Read embedded ID3 and DSDIFF names without reading the DSD programme or rewriting its file.
use anyhow::{bail,Context,Result};
use lofty::{config::ParseOptions,file::{FileType,TaggedFileExt},prelude::Accessor,probe::Probe,tag::{Tag,TagType}};
use rmusic_audio::dsd::DsdLayout;
use std::{fs::File,io::{Cursor,Read,Seek,SeekFrom},ops::Range};

const TAG_LIMIT:u64=16*1024*1024;

fn region(file:&mut File,layout:&DsdLayout,wanted:&[u8;4])->Result<Option<Range<u64>>> {
    file.seek(SeekFrom::Start(0))?;let mut kind=[0;4];file.read_exact(&mut kind)?;
    if &kind==b"DSD " {
        if wanted!=b"ID3 "{return Ok(None);}
        file.seek(SeekFrom::Start(20))?;let mut offset=[0;8];file.read_exact(&mut offset)?;
        let start=u64::from_le_bytes(offset);return Ok((start!=0).then_some(start..layout.file_end));
    }
    file.seek(SeekFrom::Start(16))?;
    for _ in 0..256 {
        let offset=file.stream_position()?;if offset==layout.file_end{return Ok(None);}
        if offset.checked_add(12).is_none_or(|end|end>layout.file_end){bail!("truncated DSDIFF metadata chunk");}
        let mut header=[0;12];file.read_exact(&mut header)?;
        let size=u64::from_be_bytes(header[4..].try_into()?);let start=offset+12;
        let end=start.checked_add(size).filter(|end|*end<=layout.file_end).context("DSDIFF metadata chunk exceeds its form")?;
        let padded=end.checked_add(size%2).filter(|end|*end<=layout.file_end).context("DSDIFF metadata padding exceeds its form")?;
        if &header[..4]==wanted{return Ok(Some(start..end));}
        file.seek(SeekFrom::Start(padded))?;
    }
    bail!("too many DSDIFF metadata chunks")
}

fn id3_tag(file:&mut File,layout:&DsdLayout)->Result<Option<Tag>> {
    let Some(region)=region(file,layout,b"ID3 ")? else{return Ok(None);};
    if region.end>file.metadata()?.len()||region.end.saturating_sub(region.start)<10 {bail!("truncated DSD ID3 tag");}
    file.seek(SeekFrom::Start(region.start))?;let mut header=[0;10];file.read_exact(&mut header)?;
    if &header[..3]!=b"ID3"||![2,3,4].contains(&header[3])||header[6..].iter().any(|byte|byte&128!=0){bail!("invalid DSD ID3 header");}
    let size=header[6..].iter().fold(0_u64,|size,byte|(size<<7)|*byte as u64);
    let length=10+size+if header[3]==4&&header[5]&16!=0{10}else{0};
    if length>TAG_LIMIT||length>region.end-region.start {bail!("DSD ID3 tag exceeds its metadata range or 16 MiB limit");}
    let mut bytes=Vec::with_capacity(length as usize+128);bytes.extend(header);
    file.take(length-10).read_to_end(&mut bytes)?;if bytes.len()!=length as usize{bail!("truncated DSD ID3 body");}
    // Lofty exposes ID3 through file readers. Disable audio properties and isolate end-tag probes in zero padding.
    bytes.extend([0;128]);
    let mut parsed=Probe::with_file_type(Cursor::new(bytes),FileType::Mpeg).options(ParseOptions::new().read_properties(false)).read()?;
    Ok(parsed.remove(TagType::Id3v2))
}

fn diin_tag(file:&mut File,layout:&DsdLayout)->Result<Option<Tag>> {
    let Some(region)=region(file,layout,b"DIIN")? else{return Ok(None);};
    if region.end>file.metadata()?.len()||region.end-region.start>TAG_LIMIT{bail!("DSDIFF DIIN exceeds its file or 16 MiB limit");}
    file.seek(SeekFrom::Start(region.start))?;let mut tag=Tag::new(TagType::Id3v2);let mut seen=[false;2];
    for _ in 0..256 {
        let offset=file.stream_position()?;if offset==region.end{return Ok(Some(tag));}
        if offset.checked_add(12).is_none_or(|end|end>region.end){bail!("truncated DSDIFF DIIN chunk");}
        let mut header=[0;12];file.read_exact(&mut header)?;let size=u64::from_be_bytes(header[4..].try_into()?);let start=offset+12;
        let end=start.checked_add(size).filter(|end|*end<=region.end).context("DSDIFF DIIN chunk exceeds its container")?;
        let padded=end.checked_add(size%2).filter(|end|*end<=region.end).context("DSDIFF DIIN padding exceeds its container")?;
        let index=match &header[..4]{b"DITI"=>Some(0),b"DIAR"=>Some(1),_=>None};
        if let Some(index)=index {
            if seen[index]{bail!("duplicate DSDIFF DIIN name");}seen[index]=true;
            if size<4{bail!("truncated DSDIFF DIIN name count");}
            let mut count=[0;4];file.read_exact(&mut count)?;let count=u32::from_be_bytes(count) as u64;
            if count>size-4{bail!("DSDIFF DIIN name exceeds its chunk");}
            let mut bytes=vec![0;count as usize];file.read_exact(&mut bytes)?;
            if bytes.iter().any(|byte|!(0x20..=0x7e).contains(byte)){bail!("DSDIFF DIIN name is not printable ASCII");}
            let name=String::from_utf8(bytes)?;
            if !name.trim().is_empty(){if index==0{tag.set_title(name);}else{tag.set_artist(name);}}
        }
        file.seek(SeekFrom::Start(padded))?;
    }
    if file.stream_position()?==region.end{return Ok(Some(tag));}
    bail!("too many DSDIFF DIIN chunks")
}

pub(super) fn tag(file:&mut File,layout:&DsdLayout)->Result<Option<Tag>> {
    let mut tag=match id3_tag(file,layout){Ok(tag)=>tag,Err(error)=>{tracing::warn!(%error,"DSD ID3 unavailable; keeping audio properties");None}};
    match diin_tag(file,layout) {
        Ok(Some(names))=>{
            let target=tag.get_or_insert_with(||Tag::new(TagType::Id3v2));
            if target.title().is_none_or(|value|value.trim().is_empty()){if let Some(value)=names.title(){target.set_title(value.into_owned());}}
            if target.artist().is_none_or(|value|value.trim().is_empty()){if let Some(value)=names.artist(){target.set_artist(value.into_owned());}}
        }
        Err(error)=>tracing::warn!(%error,"DSDIFF DIIN unavailable; keeping ID3 and audio properties"),
        Ok(None)=>{}
    }
    Ok(tag)
}

#[cfg(test)]
mod tests {
    fn syncsafe(value:usize)->[u8;4]{[(value>>21) as u8&127,(value>>14) as u8&127,(value>>7) as u8&127,value as u8&127]}
    fn chunk(kind:&[u8;4],data:&[u8])->Vec<u8>{let mut bytes=kind.to_vec();bytes.extend((data.len() as u64).to_be_bytes());bytes.extend(data);if data.len()%2!=0{bytes.push(0);}bytes}
    fn id3(version:u8,picture:&[u8])->Vec<u8> {
        let mut frames=Vec::new();
        let title=if version==4{"真实标题".as_bytes()}else{b"Recorded title"};
        for (id,value) in [(b"TIT2",title),(b"TPE1",b"Performer"),(b"TALB",b"Album"),(b"TPE2",b"Album performer"),(b"TRCK",b"7/9"),(b"TPOS",b"2/3")] {
            let mut payload=vec![if version==4{3}else{0}];payload.extend(value);frames.extend(id);
            frames.extend(if version==4{syncsafe(payload.len())}else{(payload.len() as u32).to_be_bytes()});frames.extend([0,0]);frames.extend(payload);
        }
        let lyrics=if version==4{b"\x03zho\0".iter().copied().chain("嵌入歌词".as_bytes().iter().copied()).collect()}else{b"\0eng\0Embedded lyrics".to_vec()};
        for (id,mut payload) in [(b"USLT",lyrics),(b"APIC",b"\0image/png\0\x03\0".to_vec())] {
            if id==b"APIC"{payload.extend(picture);}
            frames.extend(id);frames.extend(if version==4{syncsafe(payload.len())}else{(payload.len() as u32).to_be_bytes()});frames.extend([0,0]);frames.extend(payload);
        }
        let mut tag=b"ID3".to_vec();tag.extend([version,0,0]);tag.extend(syncsafe(frames.len()));tag.extend(frames);tag
    }
    fn source(format:&str,tag:&[u8])->Vec<u8> {
        if format=="dsf" {
            let mut bytes=b"DSD ".to_vec();bytes.extend(28_u64.to_le_bytes());bytes.extend((8284+tag.len() as u64).to_le_bytes());bytes.extend(8284_u64.to_le_bytes());bytes.extend(b"fmt ");bytes.extend(52_u64.to_le_bytes());
            for value in [1_u32,0,2,2,2822400,1]{bytes.extend(value.to_le_bytes());}
            bytes.extend(32768_u64.to_le_bytes());bytes.extend(4096_u32.to_le_bytes());bytes.extend(0_u32.to_le_bytes());bytes.extend(b"data");bytes.extend(8204_u64.to_le_bytes());bytes.extend([0x96;8192]);bytes.extend(tag);bytes
        }else{
            let mut prop=b"SND ".to_vec();prop.extend(chunk(b"FS  ",&2822400_u32.to_be_bytes()));prop.extend(chunk(b"CHNL",b"\0\x02SLFTSRGT"));prop.extend(chunk(b"CMPR",b"DSD \x03raw"));
            let mut form=b"DSD ".to_vec();form.extend(chunk(b"PROP",&prop));form.extend(chunk(b"DSD ",&[0x69;8]));form.extend(chunk(b"ID3 ",tag));
            let mut bytes=b"FRM8".to_vec();bytes.extend((form.len() as u64).to_be_bytes());bytes.extend(form);bytes
        }
    }
    #[test]
    fn dsd_tags_read_embedded_id3_text_lyrics_cover_without_changing_audio() {
        let directory=tempfile::tempdir().unwrap();std::fs::create_dir(directory.path().join("covers")).unwrap();
        let picture=data_encoding::BASE64.decode(b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jB1sAAAAASUVORK5CYII=").unwrap();
        for (format,version) in [("dsf",4),("dff",3)] {
            let bytes=source(format,&id3(version,&picture));let path=directory.path().join(format!("filename-{format}.{format}"));std::fs::write(&path,&bytes).unwrap();
            let (track,lyrics)=crate::library::probe_track(&path,"same-id",format,directory.path()).unwrap();
            assert_eq!(track.title,if version==4{"真实标题"}else{"Recorded title"});assert_eq!(track.artist,"Performer");assert_eq!(track.album,"Album");assert_eq!(track.album_artist,"Album performer");assert_eq!((track.track_number,track.disc_number),(7,2));assert_eq!(lyrics.as_deref(),Some(if version==4{"嵌入歌词"}else{"Embedded lyrics"}));
            let key=track.cover_url.unwrap().strip_prefix("/api/v1/covers/").unwrap().to_owned();assert_eq!(std::fs::read(directory.path().join("covers").join(key)).unwrap(),picture);
            assert_eq!(track.sample_rate,2822400);assert_eq!(track.bit_depth,1);assert_eq!(std::fs::read(&path).unwrap(),bytes);
            std::fs::write(path.with_extension("lrc"),"[00:00.00]Sidecar wins").unwrap();
            assert_eq!(crate::library::probe_track(&path,"same-id",format,directory.path()).unwrap().1.as_deref(),Some("[00:00.00]Sidecar wins"));
        }
    }
    #[test]
    fn dsd_tags_reject_bad_or_oversized_metadata_but_preserve_audio_properties() {
        let directory=tempfile::tempdir().unwrap();
        let path=directory.path().join("small.dsf");std::fs::write(&path,source("dsf",b"ID3\x04\0\0\0\0\0\x0cTIT2\0\0\0\x02\0\0\x03X")).unwrap();
        assert_eq!(crate::library::probe_track(&path,"same-id","dsf",directory.path()).unwrap().0.title,"X");
        for (index,tag) in [b"ID3\x04\0\0\x7f\x7f\x7f\x7f".as_slice(),b"invalid tag bytes"].into_iter().enumerate() {
            let path=directory.path().join(format!("invalid-{index}.dsf"));std::fs::write(&path,source("dsf",tag)).unwrap();
            let (track,_)=crate::library::probe_track(&path,"same-id","dsf",directory.path()).unwrap();
            assert_eq!(track.sample_rate,2822400);assert_eq!(track.bit_depth,1);assert_eq!(track.title,format!("invalid-{index}"));
        }
        let path=directory.path().join("truncated.dsf");let mut bytes=source("dsf",b"ID3\x04\0\0\0\0\0\0");bytes.truncate(100);std::fs::write(&path,bytes).unwrap();
        assert!(crate::library::probe_track(&path,"same-id","dsf",directory.path()).is_err());
    }
    fn diin_name(kind:&[u8;4],name:&[u8])->Vec<u8>{let mut text=(name.len() as u32).to_be_bytes().to_vec();text.extend(name);chunk(kind,&text)}
    fn dff_with_diin(tag:&[u8],metadata:&[u8],before:bool)->Vec<u8> {
        let mut bytes=source("dff",tag);let offset=if before{16}else{bytes.len()};bytes.splice(offset..offset,chunk(b"DIIN",metadata));
        let size=(bytes.len()-12) as u64;bytes[4..12].copy_from_slice(&size.to_be_bytes());bytes
    }
    #[test]
    fn dff_diin_names_fill_missing_id3_fields_and_keep_programme() {
        let directory=tempfile::tempdir().unwrap();std::fs::create_dir(directory.path().join("covers")).unwrap();
        let mut metadata=chunk(b"EMID",b"skip me");metadata.extend(diin_name(b"DITI",b"Project title"));metadata.extend(diin_name(b"DIAR",b"DIIN performer"));
        let empty=b"ID3\x04\0\0\0\0\0\0";
        let title_only=b"ID3\x04\0\0\0\0\0\x0cTIT2\0\0\0\x02\0\0\x03X";
        for (index,tag,before,title,artist) in [
            (0,empty.as_slice(),true,"Project title","DIIN performer"),
            (1,empty.as_slice(),false,"Project title","DIIN performer"),
            (2,title_only.as_slice(),false,"X","DIIN performer"),
            (3,b"invalid ID3".as_slice(),false,"Project title","DIIN performer"),
        ] {
            let bytes=dff_with_diin(tag,&metadata,before);let path=directory.path().join(format!("fallback-{index}.dff"));std::fs::write(&path,&bytes).unwrap();
            let (track,_)=crate::library::probe_track(&path,"same-id","dff",directory.path()).unwrap();
            assert_eq!(track.title,title);assert_eq!(track.artist,artist);assert_eq!((track.sample_rate,track.bit_depth,track.channels),(2822400,1,2));
            assert_eq!(track.provider_data["dsd_diin_v1"],true);assert_eq!(std::fs::read(&path).unwrap(),bytes);
        }
        let path=directory.path().join("id3-wins.dff");std::fs::write(&path,dff_with_diin(&id3(4,b""),&metadata,false)).unwrap();
        let (track,_)=crate::library::probe_track(&path,"same-id","dff",directory.path()).unwrap();assert_eq!(track.title,"真实标题");assert_eq!(track.artist,"Performer");
    }
    #[test]
    fn dff_diin_malformed_bounds_or_text_preserve_valid_id3_and_audio() {
        let directory=tempfile::tempdir().unwrap();std::fs::create_dir(directory.path().join("covers")).unwrap();
        let mut overflow=b"DITI".to_vec();overflow.extend(u64::MAX.to_be_bytes());
        let mut count=99_u32.to_be_bytes().to_vec();count.extend(b"short");
        let mut duplicate=diin_name(b"DITI",b"first");duplicate.extend(diin_name(b"DITI",b"second"));
        let mut cases=vec![b"DITI".to_vec(),overflow,chunk(b"DITI",&count),diin_name(b"DITI",b"bad\0text"),diin_name(b"DIAR",b"\xff"),duplicate,chunk(b"DITI",b"\0")];
        cases.push(chunk(b"JUNK",&vec![0;16*1024*1024]));
        cases.push((0..257).flat_map(|_|chunk(b"JUNK",b"")).collect());
        for (index,metadata) in cases.into_iter().enumerate() {
            let bytes=dff_with_diin(&id3(4,b""),&metadata,false);let path=directory.path().join(format!("invalid-{index}.dff"));std::fs::write(&path,&bytes).unwrap();
            let (track,_)=crate::library::probe_track(&path,"same-id","dff",directory.path()).unwrap();
            assert_eq!(track.title,"真实标题");assert_eq!(track.artist,"Performer");assert_eq!(track.sample_rate,2822400);assert_eq!(std::fs::read(&path).unwrap(),bytes);
        }
        let path=directory.path().join("empty.dff");let mut empty=diin_name(b"DITI",b"   ");empty.extend(diin_name(b"DIAR",b""));
        std::fs::write(&path,dff_with_diin(b"invalid ID3",&empty,false)).unwrap();
        let (track,_)=crate::library::probe_track(&path,"same-id","dff",directory.path()).unwrap();assert_eq!(track.title,"empty");assert_eq!(track.artist,"未知艺术家");
    }

}
