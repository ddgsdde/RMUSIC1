use super::*;
use std::{io::{Cursor,Write},net::TcpListener,sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},time::Duration};

pub(crate) fn samples(bytes:usize)->Vec<Vec<u8>> {
    (0..2).map(|channel|(0..bytes).map(|index|{
        if index==0 {[0x01,0x7f][channel]}else if index==1 {[0x80,0x40][channel]}else{(index*37+channel*53) as u8}
    }).collect()).collect()
}
pub(crate) fn dsf(data:&[Vec<u8>],count:u64,lsb:bool)->Vec<u8> {
    let groups=data[0].len().div_ceil(4096);let size=groups*4096*data.len();
    let mut bytes=b"DSD ".to_vec();bytes.extend(28_u64.to_le_bytes());bytes.extend((92+size as u64).to_le_bytes());bytes.extend(0_u64.to_le_bytes());
    bytes.extend(b"fmt ");bytes.extend(52_u64.to_le_bytes());
    for value in [1,0,2,data.len() as u32,2822400,if lsb{1}else{8}] {bytes.extend(value.to_le_bytes());}
    bytes.extend(count.to_le_bytes());bytes.extend(4096_u32.to_le_bytes());bytes.extend(0_u32.to_le_bytes());
    bytes.extend(b"data");bytes.extend((size as u64+12).to_le_bytes());
    for group in 0..groups {for channel in data {for index in group*4096..(group+1)*4096 {
        let value=channel.get(index).copied().unwrap_or(0);bytes.push(if lsb{value.reverse_bits()}else{value});
    }}}
    bytes
}
fn chunk(kind:&[u8;4],data:&[u8])->Vec<u8> {
    let mut bytes=kind.to_vec();bytes.extend((data.len() as u64).to_be_bytes());bytes.extend(data);if data.len()%2!=0{bytes.push(0);}bytes
}
pub(crate) fn dff(data:&[Vec<u8>])->Vec<u8> {
    let mut prop=b"SND ".to_vec();prop.extend(chunk(b"FS  ",&2822400_u32.to_be_bytes()));
    prop.extend(chunk(b"CHNL",&[0,2,b'S',b'L',b'F',b'T',b'S',b'R',b'G',b'T']));
    prop.extend(chunk(b"CMPR",b"DSD \x03raw"));
    let mut form=b"DSD ".to_vec();form.extend(chunk(b"FVER",&0x01050000_u32.to_be_bytes()));form.extend(chunk(b"PROP",&prop));
    let interleaved:Vec<u8>=(0..data[0].len()).flat_map(|index|data.iter().map(move|channel|channel[index])).collect();
    form.extend(chunk(b"DSD ",&interleaved));
    let mut bytes=b"FRM8".to_vec();bytes.extend((form.len() as u64).to_be_bytes());bytes.extend(form);bytes
}
fn assert_bits(output:&[u8],data:&[Vec<u8>],start:u64,count:u64,container:DopContainer) {
    let width=container.bytes();let frames=output.len()/(width*data.len());
    for frame in 0..frames {for channel in 0..data.len() {
        let sample=&output[(frame*data.len()+channel)*width..][..width];
        let word=if width==3{u32::from_le_bytes([sample[0],sample[1],sample[2],0])}else{u32::from_le_bytes(sample.try_into().unwrap())>>8};
        assert_eq!(word>>16,if frame%2==0{5}else{250});
        for bit in 0..16 {
            let source=(start+frame as u64)*16+bit;
            if source<count {
                let expected=(data[channel][source as usize/8]>>(7-source%8))&1;
                assert_eq!((word>>(15-bit))&1,expected as u32,"frame {frame}, channel {channel}, bit {bit}");
            }
        }
    }}
}

#[test]
fn dsf_and_dff_dop_preserve_bits_across_blocks_odd_reads_seeks_and_padding() {
    let data=samples(4097);
    for (file,count) in [(dsf(&data,4097*8-3,true),4097_u64*8-3),(dsf(&data,4097*8,false),4097*8),(dff(&data),4097*8)] {
        for container in [DopContainer::Packed24,DopContainer::LeftAligned32] {
            let mut reader=DsdReader::new(Cursor::new(&file)).unwrap();
            assert_eq!(reader.layout().dop_rate().unwrap(),176400);
            let layout=reader.layout().clone();let mut encoder=DopEncoder::new(2).unwrap();
            assert_eq!(layout.data_end,file.len() as u64);assert_eq!(layout.file_end,layout.data_end);
            assert_eq!(layout.frame_at(layout.duration()).unwrap(),count.div_ceil(16));
            let mut all=Vec::new();let mut output=vec![0;7*2*container.bytes()];
            loop {let n=reader.read_dop(&mut encoder,&mut output,container,false).unwrap();if n==0{break;}all.extend_from_slice(&output[..n*2*container.bytes()]);}
            assert_eq!(all.len(),count.div_ceil(16) as usize*2*container.bytes());
            assert_bits(&all,&data,0,count,container);
            if container==DopContainer::Packed24 {assert_eq!(&all[..6],&[0x80,0x01,0x05,0x40,0x7f,0x05]);}
            assert_eq!(reader.position(),count as f64/2822400.);
            let frame=2047;let mut source=Cursor::new(&file);source.set_position(layout.seek_offset(frame).unwrap());
            let mut seeked=DsdReader::at(source,layout,frame).unwrap();let mut encoder=DopEncoder::new(2).unwrap();
            let n=seeked.read_dop(&mut encoder,&mut output,container,false).unwrap();assert_eq!(n,2);
            assert_bits(&output[..n*2*container.bytes()],&data,frame,count,container);
        }
    }
    let mut with_tail=dff(&data);let audio_end=with_tail.len() as u64;
    with_tail.extend(chunk(b"ID3 ",b"metadata"));let form_size=with_tail.len() as u64-12;with_tail[4..12].copy_from_slice(&form_size.to_be_bytes());
    let layout=DsdLayout::read(&mut Cursor::new(&with_tail)).unwrap();assert_eq!(layout.data_end,audio_end);assert_eq!(layout.file_end,with_tail.len() as u64);
    let mut encoder=DopEncoder::new(2).unwrap();let mut out=[0;6];
    encoder.silence(&mut out,DopContainer::Packed24).unwrap();assert_eq!(out,[0x69,0x69,5,0x69,0x69,5]);
    encoder.pack(&[0x01,0x7f,0x80,0x40],&mut out,DopContainer::Packed24,true).unwrap();assert_eq!(out,[0x69,0x69,250,0x69,0x69,250]);
    encoder.pack(&[0x01,0x7f,0x80,0x40],&mut out,DopContainer::Packed24,false).unwrap();assert_eq!(out,[0x80,0x01,5,0x40,0x7f,5]);
}

#[test]
fn malformed_dsd_and_unsupported_carrier_fail_without_unbounded_allocation() {
    let data=samples(32);let original=dsf(&data,256,true);
    for (offset,value) in [(4,u64::MAX),(64,u64::MAX),(84,11)] {
        let mut file=original.clone();file[offset..offset+8].copy_from_slice(&value.to_le_bytes());
        assert!(DsdReader::new(Cursor::new(file)).is_err());
    }
    let mut truncated=original.clone();truncated.pop();
    let mut reader=DsdReader::new(Cursor::new(truncated)).unwrap();
    assert!(reader.read_dop(&mut DopEncoder::new(2).unwrap(),&mut[0;6],DopContainer::Packed24,false).unwrap_err().is::<DecodeFailure>());
    let mut compressed=dff(&data);let at=compressed.windows(4).position(|s|s==b"CMPR").unwrap();compressed[at+12..at+16].copy_from_slice(b"DST ");
    assert!(DsdReader::new(Cursor::new(compressed)).unwrap_err().to_string().contains("DST"));
    let mut huge=dff(&data);huge[20..28].copy_from_slice(&(HEADER_LIMIT+1).to_be_bytes());huge[4..12].copy_from_slice(&(HEADER_LIMIT+100).to_be_bytes());
    assert!(DsdReader::new(Cursor::new(huge)).unwrap_err().to_string().contains("16 MiB"));
    let mut layout=DsdReader::new(Cursor::new(original)).unwrap().layout().clone();
    for (rate,carrier) in [(2822400,176400),(5644800,352800),(11289600,705600),(12288000,768000)] {layout.sample_rate=rate;assert_eq!(layout.dop_rate().unwrap(),carrier);}
    layout.sample_rate=22579200;assert!(layout.dop_rate().is_err());
    assert!(layout.frame_at(f64::NAN).is_err());assert!(layout.frame_at(-1.).is_err());
}

#[test]
fn real_ffmpeg_byte_transport_uses_http_range_for_dsd_seek_and_is_cancellable() {
    let data=samples(4096*32);let file=Arc::new(dsf(&data,(data[0].len()*8) as u64,true));
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();listener.set_nonblocking(true).unwrap();
    let stop=Arc::new(AtomicBool::new(false));let ranges=Arc::new(Mutex::new(Vec::new()));
    let server={let stop=stop.clone();let ranges=ranges.clone();let file=file.clone();std::thread::spawn(move||{
        while !stop.load(Ordering::Relaxed) {
            let Ok((mut socket,_))=listener.accept() else {std::thread::sleep(Duration::from_millis(5));continue;};
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();socket.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut headers=Vec::new();let mut byte=[0];
            while !headers.ends_with(b"\r\n\r\n")&&headers.len()<8192 {if socket.read_exact(&mut byte).is_err(){break;}headers.push(byte[0]);}
            let headers=String::from_utf8_lossy(&headers);
            let offset=headers.lines().find_map(|line|line.strip_prefix("Range: bytes=").and_then(|s|s.split('-').next()?.parse::<usize>().ok())).unwrap_or(0);
            ranges.lock().unwrap().push(offset);
            if offset>=file.len(){let _=socket.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");continue;}
            let ignore=headers.starts_with("GET /ignore.dsf ");
            let (header,start)=if ignore {(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",file.len()),0)}else{(format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {offset}-{}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",file.len()-offset,file.len()-1,file.len()),offset)};
            if socket.write_all(header.as_bytes()).is_ok(){let _=socket.write_all(&file[start..]);}
        }
    })};
    let frame=49155_u64;let position=frame as f64/176400.;
    let result=(||->Result<()> {
        let mut source=open(&format!("http://{address}/source.dsf"),position,|_|Ok(()))?;
        let mut output=[0;42];assert_eq!(source.read_dop(&mut DopEncoder::new(2)?,&mut output,DopContainer::Packed24,false)?,7);
        assert_bits(&output,&data,frame,(data[0].len()*8) as u64,DopContainer::Packed24);
        assert_eq!(*ranges.lock().unwrap(),vec![0,196700],"seek must start directly at the requested byte Range");
        let mut ignored=open(&format!("http://{address}/ignore.dsf"),position,|_|Ok(()))?;
        assert!(ignored.read_dop(&mut DopEncoder::new(2)?,&mut output,DopContainer::Packed24,false).is_err(),"ignored Range must not replay byte zero");
        Ok(())
    })();
    stop.store(true,Ordering::Relaxed);server.join().unwrap();result.unwrap();
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
    let (connected,connection)=std::sync::mpsc::channel();let (release,hold)=std::sync::mpsc::channel();
    let server=std::thread::spawn(move||{let (socket,_)=listener.accept().unwrap();connected.send(()).unwrap();let _=hold.recv_timeout(Duration::from_secs(10));drop(socket);});
    let (handle,abort)=std::sync::mpsc::channel();let (done,finished)=std::sync::mpsc::channel();
    let worker=std::thread::spawn(move||{let result=open(&format!("http://{address}/blocked.dsf"),0.,|abort|{handle.send(abort).unwrap();Ok(())});done.send(result.is_err()).unwrap();});
    let abort=abort.recv_timeout(Duration::from_secs(5)).unwrap();connection.recv_timeout(Duration::from_secs(5)).unwrap();abort.abort();
    let cancelled=finished.recv_timeout(Duration::from_secs(2));let _=release.send(());server.join().unwrap();worker.join().unwrap();assert!(cancelled.unwrap());
    println!("DSD/DoP: DSF raw HTTP Range seek at frame {frame}, source bits preserved; blocked header cancelled; no device opened");
}
