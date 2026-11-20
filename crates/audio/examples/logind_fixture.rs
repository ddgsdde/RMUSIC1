//! Development fixture: owns login1 only on a newly created private bus.
#[cfg(all(target_os="linux",feature="alsa-output"))]
#[path="../tests/support/logind.rs"] mod logind;
#[cfg(all(target_os="linux",feature="alsa-output"))]
fn main() {
    use std::io::{BufRead,Read,Write};
    let bus=logind::Bus::new();let manager=logind::Manager::new(&bus);let mut lock=None;
    println!("{}",serde_json::json!({"address":bus.address}));std::io::stdout().flush().unwrap();
    for command in std::io::stdin().lock().lines(){
        let command=command.unwrap();let reply=match command.as_str(){
            "lock"=>{let stream=manager.lock();stream.set_nonblocking(true).unwrap();lock=Some(stream);serde_json::json!({"locked":true})},
            "sleep"=>{manager.sleep(true);serde_json::json!({"sent":true})},
            "wake"=>{manager.sleep(false);serde_json::json!({"sent":true})},
            "released"=>serde_json::json!({"released":lock.as_mut().is_some_and(|stream|matches!(stream.read(&mut [0;1]),Ok(0)))}),
            "quit"=>break,
            _=>panic!("unknown fixture command")
        };println!("{reply}");std::io::stdout().flush().unwrap();
    }
}
#[cfg(not(all(target_os="linux",feature="alsa-output")))]
fn main(){panic!("This private fixture requires Linux and alsa-output");}
