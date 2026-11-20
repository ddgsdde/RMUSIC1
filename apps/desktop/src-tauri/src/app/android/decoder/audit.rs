//! Debug-only source contracts, without opening any audio device.
use super::*;
pub(crate) fn seek(url:&str)->Result<Vec<u8>> {
    let mut input=Input::open(url,cancellation())?;input.prepare()?;
    let mut bytes=vec![0;48];input.read_exact(&mut bytes[..16])?;
    input.seek(SeekFrom::End(-16))?;input.read_exact(&mut bytes[16..32])?;
    input.seek(SeekFrom::Start(7))?;input.read_exact(&mut bytes[32..])?;
    Ok(bytes)
}
pub(crate) fn cancel_seek(body_started:bool)->Result<u128> {
    use std::{io::{BufRead,BufReader,Write},net::{TcpListener,TcpStream},time::Instant};
    fn accept(server:&TcpListener)->Result<TcpStream> {
        let deadline=Instant::now()+Duration::from_secs(6);
        loop {match server.accept(){Ok((socket,_))=>return Ok(socket),Err(error) if error.kind()==io::ErrorKind::WouldBlock=>{},Err(error)=>return Err(error.into())}
            if Instant::now()>=deadline{bail!("HTTP seek fixture connection timed out");}std::thread::sleep(Duration::from_millis(5));}
    }
    fn request(socket:&mut TcpStream)->Result<String> {
        socket.set_read_timeout(Some(Duration::from_secs(2)))?;let mut reader=BufReader::new(socket);let mut text=String::new();
        loop {let mut line=String::new();if reader.read_line(&mut line)?==0{bail!("HTTP seek fixture request closed");}text.push_str(&line);if line=="\r\n"{return Ok(text);}}
    }
    let server=TcpListener::bind("127.0.0.1:0")?;server.set_nonblocking(true)?;let address=server.local_addr()?;
    let (accepted,connection)=std::sync::mpsc::channel();let (release,wait)=std::sync::mpsc::channel();
    let server_thread=std::thread::spawn(move||->Result<bool>{
        let mut first=accept(&server)?;request(&mut first)?;
        first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 262144\r\nAccept-Ranges: bytes\r\nETag: \"seek-v1\"\r\nConnection: close\r\n\r\n")?;
        first.write_all(&[0;64])?;
        let mut second=accept(&server)?;let headers=request(&mut second)?.to_lowercase();
        if !headers.contains("range: bytes=262128-")||!headers.contains("if-range: \"seek-v1\""){bail!("missing guarded seek headers");}
        if body_started {second.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 16\r\nContent-Range: bytes 262128-262143/262144\r\nETag: \"seek-v1\"\r\nConnection: close\r\n\r\n")?;}
        let _=accepted.send(());let _=wait.recv_timeout(Duration::from_secs(6));
        first.set_read_timeout(Some(Duration::from_secs(1)))?;second.set_read_timeout(Some(Duration::from_secs(1)))?;
        let mut byte=[0];Ok(first.read(&mut byte).is_ok_and(|n|n==0)&&second.read(&mut byte).is_ok_and(|n|n==0))
    });
    let cancel=cancellation();let worker_cancel=cancel.clone();
    let worker=std::thread::spawn(move||->Result<()>{
        let mut input=Input::open(&format!("http://{address}/seek"),worker_cancel)?;input.prepare()?;
        let mut bytes=[0;16];input.read_exact(&mut bytes)?;input.seek(SeekFrom::End(-16))?;input.read_exact(&mut bytes)?;Ok(())
    });
    let connected=connection.recv_timeout(Duration::from_secs(6));
    if connected.is_ok(){std::thread::sleep(Duration::from_millis(100));}
    let start=Instant::now();cancel.stop();let stopped=worker.join();let elapsed=start.elapsed().as_millis();
    let _=release.send(());let closed=server_thread.join().map_err(|_|anyhow::anyhow!("HTTP fixture panicked"))??;
    connected.context("HTTP seek did not reach the blocked response")?;
    if stopped.map_err(|_|anyhow::anyhow!("HTTP seek worker panicked"))?.is_ok()||!closed{bail!("HTTP seek cancellation failed to release both responses");}
    Ok(elapsed)
}
pub(crate) fn cancel_platform(path:&str,body_started:bool)->Result<u128> {
    use std::{io::{BufRead,BufReader,Write},net::{TcpListener,TcpStream},time::Instant};
    let data=std::fs::read(path)?;if data.len()>256*1024{bail!("platform audit source exceeds its budget");}
    let server=TcpListener::bind("127.0.0.1:0")?;server.set_nonblocking(true)?;let address=server.local_addr()?;
    let (accepted,connection)=std::sync::mpsc::channel();let(release,wait)=std::sync::mpsc::channel();
    let server_thread=std::thread::spawn(move||->Result<bool>{
        let accept=||->Result<TcpStream>{let deadline=Instant::now()+Duration::from_secs(6);loop {
            match server.accept(){Ok((socket,_))=>return Ok(socket),Err(error) if error.kind()==io::ErrorKind::WouldBlock=>{},Err(error)=>return Err(error.into())}
            if Instant::now()>=deadline{bail!("platform callback did not connect");}std::thread::sleep(Duration::from_millis(5));
        }};
        let request=|socket:&mut TcpStream|->Result<String>{
            socket.set_read_timeout(Some(Duration::from_secs(2)))?;let mut reader=BufReader::new(socket);let mut text=String::new();
            loop {let mut line=String::new();if reader.read_line(&mut line)?==0{bail!("platform callback request closed");}text.push_str(&line);if line=="\r\n"{return Ok(text.to_lowercase());}}
        };
        let mut first=accept()?;request(&mut first)?;
        first.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: \"platform-v1\"\r\nConnection: close\r\n\r\n",data.len()).as_bytes())?;first.write_all(&data)?;
        let mut previous=vec![first];
        let mut second=loop {
            let mut socket=accept()?;let headers=request(&mut socket)?;
            if !headers.contains("range: bytes=0-")||!headers.contains("if-range: \"platform-v1\""){bail!("platform callback lacks guarded rewind");}
            if platform::active()>0 {break socket;}
            // Symphonia can rewind while probing, before MediaDataSource exists.
            if previous.len()>=8 {bail!("platform source was never registered");}
            socket.write_all(format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nETag: \"platform-v1\"\r\nConnection: close\r\n\r\n",data.len(),data.len()-1,data.len()).as_bytes())?;socket.write_all(&data)?;previous.push(socket);
        };
        if body_started {second.write_all(format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nETag: \"platform-v1\"\r\nConnection: close\r\n\r\n",data.len(),data.len()-1,data.len()).as_bytes())?;}
        let _=accepted.send(());let _=wait.recv_timeout(Duration::from_secs(6));
        second.set_read_timeout(Some(Duration::from_secs(1)))?;let mut byte=[0];let mut closed=second.read(&mut byte).is_ok_and(|n|n==0);
        for socket in &mut previous {socket.set_read_timeout(Some(Duration::from_secs(1)))?;closed &= socket.read(&mut byte).is_ok_and(|n|n==0);}
        Ok(closed)
    });
    let control=Arc::new(PlaybackControl::default());let worker_control=control.clone();
    let worker=std::thread::spawn(move||output::android::audit_queue(vec![PlaybackRequest{flac_samples: None, dsd_pcm: None, input:format!("http://{address}/platform.m4a"),position:0.,sample_rate:48000,channels:2,source_bit_depth:0,source_integer_lossless:false,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:None,transitions:Default::default()}],worker_control,false));
    let connected=connection.recv_timeout(Duration::from_secs(6));let in_callback=platform::active()>0;
    if connected.is_ok(){std::thread::sleep(Duration::from_millis(100));}
    let start=Instant::now();control.stop();let stopped=worker.join();let elapsed=start.elapsed().as_millis();
    let _=release.send(());let closed=server_thread.join().map_err(|_|anyhow::anyhow!("platform fixture panicked"))??;
    connected.context("platform decoder did not reach its source callback")?;
    let stopped=stopped.map_err(|_|anyhow::anyhow!("platform decoder panicked"))?;
    let programme=stopped.is_ok_and(|bytes|!bytes.is_empty());let active=platform::active();
    if !in_callback||!closed||programme||active!=0 {bail!("platform callback cleanup: in_callback={in_callback}, closed={closed}, programme={programme}, active={active}, elapsed={elapsed}");}
    Ok(elapsed)
}
