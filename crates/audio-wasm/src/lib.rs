//! Browser ABI over the exact native pure DSP modules, without native I/O or Tokio.
use anyhow::{bail,Context,Result};
use std::{cell::RefCell,collections::HashMap};
#[path="../../audio/src/convolution.rs"]
pub mod convolution;
#[path="../../audio/src/impulse.rs"]
pub mod impulse;
#[path="../../audio/src/dsp.rs"]
pub mod dsp;
pub use dsp::{DspConfig,DspProcessor,Meter};

const MAX_FRAMES:usize=4096;
struct Processor {dsp:DspProcessor,channels:usize,samples:Vec<f64>,envelope:Vec<f64>,meter:Meter}
#[derive(Default)]
struct State {next:u32,error:String,buffers:HashMap<u32,Vec<u8>>,responses:HashMap<u32,impulse::ImpulseResponse>,processor:Option<(u32,Processor)>}
thread_local! {static STATE:RefCell<State>=RefCell::new(State::default());}
impl State {
    fn id(&mut self)->Result<u32>{self.next=self.next.checked_add(1).context("WASM handle limit")?;Ok(self.next)}
    fn buffer(&self,id:u32)->Result<&Vec<u8>>{self.buffers.get(&id).context("invalid WASM buffer")}
    fn response(&self,id:u32)->Result<&impulse::ImpulseResponse>{self.responses.get(&id).context("invalid WASM response")}
    fn processor(&mut self,id:u32)->Result<&mut Processor>{self.processor.as_mut().filter(|(key,_)|*key==id).map(|(_,value)|value).context("invalid WASM processor")}
}
fn operate<T>(failure:T,action:impl FnOnce(&mut State)->Result<T>)->T {
    STATE.with(|state|{let mut state=state.borrow_mut();state.error.clear();match action(&mut state){Ok(value)=>value,Err(error)=>{state.error=error.to_string();failure}}})
}
#[no_mangle]
pub extern "C" fn rm_error_len()->usize{STATE.with(|state|state.borrow().error.len())}
#[no_mangle]
pub extern "C" fn rm_error_ptr()->usize{STATE.with(|state|state.borrow().error.as_ptr() as usize)}
#[no_mangle]
pub extern "C" fn rm_buffer_new(size:usize)->u32{operate(0,|state|{
    if size==0||size>impulse::MAX_WAV_BYTES||state.buffers.len()>=16||state.buffers.values().map(Vec::len).sum::<usize>()+size>impulse::MAX_BANK_BYTES{bail!("WASM buffer budget exceeded");}
    let id=state.id()?;state.buffers.insert(id,vec![0;size]);Ok(id)
})}
#[no_mangle]
pub extern "C" fn rm_buffer_ptr(id:u32)->usize{operate(0,|state|Ok(state.buffer(id)?.as_ptr() as usize))}
#[no_mangle]
pub extern "C" fn rm_buffer_free(id:u32)->u32{operate(0,|state|{state.buffers.remove(&id).context("invalid WASM buffer")?;Ok(1)})}
#[no_mangle]
pub extern "C" fn rm_response_new(buffer:u32)->u32{operate(0,|state|{
    let response=impulse::ImpulseResponse::from_wav(state.buffer(buffer)?)?;
    if state.responses.len()>=impulse::MAX_BANK_RESPONSES||state.responses.values().map(|value|value.frames()*value.channels()).sum::<usize>()+response.frames()*response.channels()>impulse::MAX_BANK_COEFFICIENTS{bail!("WASM response bank budget exceeded");}
    let id=state.id()?;state.responses.insert(id,response);Ok(id)
})}
#[no_mangle]
pub extern "C" fn rm_response_rate(id:u32)->u32{operate(0,|state|Ok(state.response(id)?.sample_rate()))}
#[no_mangle]
pub extern "C" fn rm_response_channels(id:u32)->usize{operate(0,|state|Ok(state.response(id)?.channels()))}
#[no_mangle]
pub extern "C" fn rm_response_frames(id:u32)->usize{operate(0,|state|Ok(state.response(id)?.frames()))}
#[no_mangle]
pub extern "C" fn rm_response_mask(id:u32)->u32{operate(0,|state|Ok(state.response(id)?.channel_mask()))}
#[no_mangle]
pub extern "C" fn rm_response_channel_ptr(id:u32,channel:usize)->usize{operate(0,|state|Ok(state.response(id)?.kernels().get(channel).context("invalid WASM channel")?.as_ptr() as usize))}
#[no_mangle]
pub extern "C" fn rm_response_free(id:u32)->u32{operate(0,|state|{state.responses.remove(&id).context("invalid WASM response")?;Ok(1)})}
#[no_mangle]
pub extern "C" fn rm_processor_new(config:u32,rate:u32,channels:usize,mask:u32,response:u32,measured:f64)->u32{operate(0,|state|{
    if state.processor.is_some(){bail!("WASM processor already exists");}
    let bytes=state.buffer(config)?;if bytes.len()>65536{bail!("WASM DSP configuration too large");}
    let config:DspConfig=serde_json::from_slice(bytes)?;
    if config.enabled&&config.convolution.is_some()&&response==0{bail!("卷积响应尚未加载");}
    let mut dsp=DspProcessor::new(config,rate,channels,measured.is_finite().then_some(measured))?;
    if response!=0 {let response=state.response(response)?;response.validate_output(rate,channels as u16,Some(mask))?;dsp.attach_convolution(rate,response.kernels().to_vec())?;}
    let id=state.id()?;state.processor=Some((id,Processor{dsp,channels,samples:vec![0.;MAX_FRAMES*channels],envelope:vec![1.;MAX_FRAMES],meter:Meter::default()}));Ok(id)
})}
#[no_mangle]
pub extern "C" fn rm_processor_samples_ptr(id:u32)->usize{operate(0,|state|Ok(state.processor(id)?.samples.as_ptr() as usize))}
#[no_mangle]
pub extern "C" fn rm_processor_envelope_ptr(id:u32)->usize{operate(0,|state|Ok(state.processor(id)?.envelope.as_ptr() as usize))}
#[no_mangle]
pub extern "C" fn rm_processor_process(id:u32,frames:usize)->u32{operate(0,|state|{
    if frames>MAX_FRAMES{bail!("WASM audio block too large");}
    let processor=state.processor(id)?;let envelope=&processor.envelope;
    processor.meter=processor.dsp.process_enveloped(&mut processor.samples[..frames*processor.channels],|frame|envelope[frame])?;Ok(1)
})}
#[no_mangle]
pub extern "C" fn rm_processor_tail(id:u32)->usize{operate(usize::MAX,|state|Ok(state.processor(id)?.dsp.convolution_tail_frames()))}
#[no_mangle]
pub extern "C" fn rm_processor_drain(id:u32,frames:usize)->usize{operate(usize::MAX,|state|{
    if frames>MAX_FRAMES{bail!("WASM audio block too large");}
    let processor=state.processor(id)?;let(samples,meter)=processor.dsp.drain_convolution(frames);processor.samples[..samples.len()].copy_from_slice(&samples);processor.meter=meter;Ok(samples.len()/processor.channels)
})}
#[no_mangle]
pub extern "C" fn rm_processor_peak(id:u32)->f64{operate(f64::NAN,|state|Ok(state.processor(id)?.meter.peak))}
#[no_mangle]
pub extern "C" fn rm_processor_clipped(id:u32)->f64{operate(f64::NAN,|state|Ok(state.processor(id)?.meter.clipped_samples as f64))}
#[no_mangle]
pub extern "C" fn rm_processor_loudness(id:u32,measured:f64)->u32{operate(0,|state|{state.processor(id)?.dsp.update_loudness(measured.is_finite().then_some(measured));Ok(1)})}
#[no_mangle]
pub extern "C" fn rm_processor_free(id:u32)->u32{operate(0,|state|{state.processor(id)?;state.processor=None;Ok(1)})}

#[cfg(test)]
mod tests {
    use super::*;
    fn buffer(bytes:&[u8])->u32{let id=rm_buffer_new(bytes.len());assert_ne!(id,0);STATE.with(|state|state.borrow_mut().buffers.get_mut(&id).unwrap().copy_from_slice(bytes));id}
    fn wav()->Vec<u8>{let mut bytes=b"RIFF".to_vec();bytes.extend(60_u32.to_le_bytes());bytes.extend(b"WAVEfmt ");bytes.extend(16_u32.to_le_bytes());bytes.extend(3_u16.to_le_bytes());bytes.extend(1_u16.to_le_bytes());bytes.extend(48000_u32.to_le_bytes());bytes.extend(384000_u32.to_le_bytes());bytes.extend(8_u16.to_le_bytes());bytes.extend(64_u16.to_le_bytes());bytes.extend(b"data");bytes.extend(24_u32.to_le_bytes());for value in [2_f64,-3.,-0.]{bytes.extend(value.to_le_bytes());}bytes}
    #[test]
    fn abi_matches_direct_original_gain_envelope_and_full_tail_without_unsafe_handles(){
        let input=buffer(&wav());let response=rm_response_new(input);assert_ne!(response,0);assert_eq!((rm_response_rate(response),rm_response_channels(response),rm_response_frames(response)),(48000,1,3));assert_eq!(rm_response_mask(response),0);assert_eq!(rm_buffer_free(input),1);
        let config=buffer(br#"{"enabled":true}"#);assert_eq!(rm_processor_new(config,96000,2,3,response,f64::NAN),0);let processor=rm_processor_new(config,48000,2,3,response,f64::NAN);assert_ne!(processor,0);assert_eq!(rm_response_free(response),1);assert_eq!(rm_buffer_free(config),1);
        STATE.with(|state|{let mut state=state.borrow_mut();let processor=state.processor(processor).unwrap();processor.samples[..4].copy_from_slice(&[1.,-1.,0.5,-0.5]);processor.envelope[..2].copy_from_slice(&[1.,0.5]);});assert_eq!(rm_processor_process(processor,2),1);
        STATE.with(|state|assert_eq!(state.borrow_mut().processor(processor).unwrap().samples[..4],[2.,-2.,-2.5,2.5]));assert_eq!(rm_processor_tail(processor),2);assert_eq!(rm_processor_drain(processor,128),2);STATE.with(|state|assert_eq!(state.borrow_mut().processor(processor).unwrap().samples[..4],[-0.75,0.75,0.,0.]));assert_eq!(rm_processor_tail(processor),0);assert_eq!(rm_processor_free(processor),1);assert_eq!(rm_processor_free(processor),0);assert_eq!(rm_response_channel_ptr(response,0),0);
    }
    #[test]
    fn abi_bounds_buffers_and_rejects_unloaded_configuration_before_processing(){
        assert_eq!(rm_buffer_new(0),0);assert_eq!(rm_buffer_new(impulse::MAX_WAV_BYTES+1),0);assert_eq!(rm_buffer_ptr(999),0);assert!(rm_error_len()>0);
        let config=buffer(format!("{{\"enabled\":true,\"convolution\":{{\"response_ids\":[\"{}\"]}}}}","0".repeat(64)).as_bytes());assert_eq!(rm_processor_new(config,48000,2,3,0,f64::NAN),0);assert_eq!(rm_buffer_free(config),1);
        let mut buffers=Vec::new();for _ in 0..8{buffers.push(rm_buffer_new(impulse::MAX_WAV_BYTES));}assert!(buffers.iter().all(|id|*id!=0));assert_eq!(rm_buffer_new(1),0);for id in buffers{assert_eq!(rm_buffer_free(id),1);}
    }
}
