//! Private D-Bus fixture. It never connects to the host system bus or requests real sleep.
use dbus::{arg::{OwnedFd,Variant},blocking::Connection,channel::{MatchingReceiver,Sender},message::MatchRule,Message};
use std::{io::BufRead,os::fd::IntoRawFd,os::unix::net::UnixStream,process::{Child,Command,Stdio},sync::{atomic::{AtomicBool,Ordering},mpsc,Arc},thread::JoinHandle,time::Duration};
pub const SERVICE:&str="org.freedesktop.login1";
pub const PATH:&str="/org/freedesktop/login1";
pub const INTERFACE:&str="org.freedesktop.login1.Manager";
pub struct Bus {pub address:String,daemon:Option<Child>}
impl Bus {
    pub fn new()->Self {
        if let Ok(address)=std::env::var("RMUSIC_QA_BUS_ADDRESS"){return Self{address,daemon:None};}
        let mut daemon=Command::new("dbus-daemon").args(["--session","--nofork","--print-address=1"]).stdout(Stdio::piped()).spawn().unwrap();
        let mut address=String::new();std::io::BufReader::new(daemon.stdout.take().unwrap()).read_line(&mut address).unwrap();assert!(address.starts_with("unix:"));
        Self {address:address.trim().into(),daemon:Some(daemon)}
    }
    pub fn forge(&self,value:bool) {let connection=Connection::new_address(&self.address).unwrap();connection.send(Message::new_signal(PATH,INTERFACE,"PrepareForSleep").unwrap().append1(value)).unwrap();connection.channel().flush();}
}
impl Drop for Bus {fn drop(&mut self){if let Some(child)=self.daemon.as_mut(){let _=child.kill();let _=child.wait();}}}
enum Action {Sleep(bool),Stop}
pub struct Manager {send:mpsc::Sender<Action>,locks:mpsc::Receiver<UnixStream>,thread:Option<JoinHandle<()>>}
impl Manager {
    pub fn new(bus:&Bus)->Self {
        let address=bus.address.clone();let (send,actions)=mpsc::channel();let (locked,locks)=mpsc::channel();let (started,ready)=mpsc::channel();
        let thread=std::thread::spawn(move||{
            let connection=Connection::new_address(&address).unwrap();connection.request_name(SERVICE,false,false,true).unwrap();
            let preparing=Arc::new(AtomicBool::new(false));let query=preparing.clone();
            connection.start_receive(MatchRule::new_method_call(),Box::new(move|message,c|{
                let member=message.member().map(|m|m.to_string()).unwrap_or_default();
                if member=="Inhibit" {
                    let (what,_,_,mode)=message.read4::<String,String,String,String>().unwrap();assert_eq!(what,"sleep");assert_eq!(mode,"delay");
                    let (client,peer)=UnixStream::pair().unwrap();locked.send(peer).unwrap();
                    let fd=unsafe{OwnedFd::new(client.into_raw_fd())};c.send(message.method_return().append1(fd)).unwrap();c.channel().flush();
                }else if member=="Get" {
                    let (interface,property)=message.read2::<String,String>().unwrap();assert_eq!(interface,INTERFACE);
                    let reply=match property.as_str(){"PreparingForSleep"=>message.method_return().append1(Variant(query.load(Ordering::Acquire))),"InhibitDelayMaxUSec"=>message.method_return().append1(Variant(2_000_000u64)),_=>panic!("unknown property {property}")};c.send(reply).unwrap();c.channel().flush();
                }else {panic!("unexpected fixture method {member}");}true
            }));started.send(()).unwrap();
            loop {
                connection.process(Duration::from_millis(5)).unwrap();
                while let Ok(action)=actions.try_recv(){match action{Action::Stop=>return,Action::Sleep(value)=>{
                    preparing.store(value,Ordering::Release);connection.send(Message::new_signal(PATH,INTERFACE,"PrepareForSleep").unwrap().append1(value)).unwrap();connection.channel().flush();
                }}}
            }
        });ready.recv_timeout(Duration::from_secs(3)).unwrap();Self{send,locks,thread:Some(thread)}
    }
    pub fn sleep(&self,value:bool){self.send.send(Action::Sleep(value)).unwrap();}
    pub fn lock(&self)->UnixStream {self.locks.recv_timeout(Duration::from_secs(3)).unwrap()}
}
impl Drop for Manager {fn drop(&mut self){let _=self.send.send(Action::Stop);if let Some(thread)=self.thread.take(){thread.join().unwrap();}}}
pub fn wait(mut ready:impl FnMut()->bool){let deadline=std::time::Instant::now()+Duration::from_secs(5);while !ready(){assert!(std::time::Instant::now()<deadline,"fixture wait exceeded 5 seconds");std::thread::sleep(Duration::from_millis(5));}}
