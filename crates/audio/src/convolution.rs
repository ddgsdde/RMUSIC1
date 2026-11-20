//! Streaming f64 FIR convolution: a direct head and capped, partitioned FFT tails.
//! A tail starting at tap B uses B-frame blocks; later uniform partitions use input
//! history. This adds no algorithmic delay and does not reset at caller boundaries.
use anyhow::{bail,Result};
use realfft::{num_complex::Complex,ComplexToReal,RealFftPlanner,RealToComplex};
use std::sync::Arc;

const HEAD:usize=64;
const MAX_PARTITION:usize=4096;
pub const MAX_COEFFICIENTS:usize=262144;
pub(crate) const MAX_EXPANDED_COEFFICIENTS:usize=1048576;
const MAX_GAIN_BOUND:f64=1_000_000.;

pub(crate) fn validate_coefficients(kernels:&[Vec<f64>])->Result<usize> {
    if kernels.is_empty()||kernels.len()>32||kernels.iter().map(Vec::len).sum::<usize>()>MAX_COEFFICIENTS
        ||kernels.iter().any(|k|k.is_empty()||k.iter().any(|v|!v.is_finite())||k.iter().map(|v|v.abs()).sum::<f64>()>MAX_GAIN_BOUND) {
        bail!("invalid FIR coefficients");
    }
    Ok(kernels.iter().map(Vec::len).max().unwrap())
}

#[derive(Clone)]
struct Partition {
    size:usize,
    cursor:usize,
    responses:Arc<Vec<Vec<Complex<f64>>>>,
    history:Vec<Vec<Complex<f64>>>,
    slot:usize,
    buffer:Vec<f64>,
    input_spectrum:Vec<Complex<f64>>,
    spectrum:Vec<Complex<f64>>,
    scratch:Vec<Complex<f64>>,
    output:Vec<f64>,
    overlap:Vec<f64>,
    forward:Arc<dyn RealToComplex<f64>>,
    inverse:Arc<dyn ComplexToReal<f64>>,
}
impl Partition {
    fn new(taps:&[f64],size:usize,planner:&mut RealFftPlanner<f64>)->Self {
        let forward=planner.plan_fft_forward(size*2);let inverse=planner.plan_fft_inverse(size*2);
        let mut scratch=vec![Complex::default();forward.get_scratch_len().max(inverse.get_scratch_len())];
        let responses:Vec<_>=taps.chunks(size).map(|taps|{
            let mut input=vec![0.;size*2];input[..taps.len()].copy_from_slice(taps);
            let mut response=vec![Complex::default();size+1];
            forward.process_with_scratch(&mut input,&mut response,&mut scratch).expect("planned FIR transform buffers");response
        }).collect();
        let history=vec![vec![Complex::default();size+1];responses.len()];
        Self{size,cursor:0,responses:Arc::new(responses),history,slot:0,buffer:vec![0.;size*2],input_spectrum:vec![Complex::default();size+1],spectrum:vec![Complex::default();size+1],scratch,output:vec![0.;size],overlap:vec![0.;size],forward,inverse}
    }
    fn add(&mut self,input:&[f64],output:&mut [f64]) {
        let mut offset=0;
        while offset<input.len() {
            let count=(self.size-self.cursor).min(input.len()-offset);
            self.buffer[self.cursor..self.cursor+count].copy_from_slice(&input[offset..offset+count]);
            for (sample,tail) in output[offset..offset+count].iter_mut().zip(&self.output[self.cursor..self.cursor+count]) {*sample+=tail;}
            offset+=count;self.cursor+=count;
            if self.cursor!=self.size {continue;}
            self.forward.process_with_scratch(&mut self.buffer,&mut self.input_spectrum,&mut self.scratch).expect("planned FIR transform buffers");
            self.history[self.slot].copy_from_slice(&self.input_spectrum);self.spectrum.fill(Complex::default());
            for (partition,response) in self.responses.iter().enumerate() {
                let previous=(self.slot+self.history.len()-partition)%self.history.len();
                for (output,(input,kernel)) in self.spectrum.iter_mut().zip(self.history[previous].iter().zip(response)) {*output+=input*kernel;}
            }
            // These two real-only bins must have exact zero imaginary components.
            self.spectrum[0].im=0.;self.spectrum[self.size].im=0.;
            self.inverse.process_with_scratch(&mut self.spectrum,&mut self.buffer,&mut self.scratch).expect("planned FIR transform buffers");
            let scale=1./(self.size*2) as f64;
            for i in 0..self.size {
                self.output[i]=self.buffer[i]*scale+self.overlap[i];
                self.overlap[i]=self.buffer[i+self.size]*scale;
            }
            self.buffer.fill(0.);self.cursor=0;self.slot=(self.slot+1)%self.history.len();
        }
    }
}

#[derive(Clone)]
struct Channel {
    head:Vec<f64>,
    history:Vec<f64>,
    cursor:usize,
    partitions:Vec<Partition>,
}
impl Channel {
    fn new(taps:&[f64],planner:&mut RealFftPlanner<f64>)->Self {
        let mut head=taps[..taps.len().min(HEAD)].to_vec();head.reverse();let history=vec![0.;head.len()*2];
        let mut partitions=Vec::new();let mut offset=HEAD;
        while offset<taps.len(){
            if offset==MAX_PARTITION {partitions.push(Partition::new(&taps[offset..],offset,planner));break;}
            partitions.push(Partition::new(&taps[offset..taps.len().min(offset*2)],offset,planner));offset*=2;
        }
        Self{head,history,cursor:0,partitions}
    }
    fn head_sample(&mut self,input:f64)->f64 {
        let length=self.head.len();self.history[self.cursor]=input;self.history[self.cursor+length]=input;
        // Mirrored history makes the dot product contiguous, without a wrap branch
        // per tap. Independent sums avoid one long floating-point dependency chain.
        let mut values=self.history[self.cursor+1..self.cursor+length+1].chunks_exact(4);
        let mut coefficients=self.head.chunks_exact(4);let mut sums=[0.;4];
        for (values,coefficients) in values.by_ref().zip(coefficients.by_ref()) {
            for index in 0..4 {sums[index]+=values[index]*coefficients[index];}
        }
        let mut result=sums.into_iter().sum::<f64>();
        for (value,coefficient) in values.remainder().iter().zip(coefficients.remainder()) {result+=value*coefficient;}
        self.cursor=(self.cursor+1)%self.head.len();
        result
    }
    fn process(&mut self,input:&[f64],output:&mut [f64]) {
        for (sample,value) in output.iter_mut().zip(input) {*sample=self.head_sample(*value);}
        for partition in &mut self.partitions {partition.add(input,output);}
    }
}

/// One mono kernel can apply independently to all output channels; otherwise one
/// kernel per channel is required. No normalization, crossmix or sample-rate conversion.
pub struct FirConvolver {
    channels:Vec<Channel>,
    tail:usize,
    remaining:usize,
    input:Vec<f64>,
    output:Vec<f64>,
}
impl FirConvolver {
    pub fn new(kernels:Vec<Vec<f64>>,channels:usize)->Result<Self> {
        if channels==0||channels>32||!(kernels.len()==1||kernels.len()==channels) {bail!("invalid FIR channel mapping");}
        let longest=validate_coefficients(&kernels)?;
        if longest*channels>MAX_EXPANDED_COEFFICIENTS {bail!("FIR exceeds the output channel processing budget");}
        let mut planner=RealFftPlanner::new();
        let mut filters=if kernels.len()==1 {vec![Channel::new(&kernels[0],&mut planner);channels]}
            else{kernels.iter().map(|k|Channel::new(k,&mut planner)).collect()};
        // Start blocks in preceding zero input at different phases. Each partition
        // still delays by its tap offset, while channels avoid simultaneous FFT work.
        for (index,filter) in filters.iter_mut().enumerate() {
            for partition in &mut filter.partitions {partition.cursor=index*partition.size/channels;}
        }
        Ok(Self{channels:filters,tail:longest-1,remaining:0,input:vec![0.;MAX_PARTITION],output:vec![0.;MAX_PARTITION]})
    }
    fn apply(&mut self,samples:&mut [f64]) {
        let channels=self.channels.len();
        for block in samples.chunks_mut(MAX_PARTITION*channels) {
            let frames=block.len()/channels;
            for (channel,filter) in self.channels.iter_mut().enumerate() {
                let input=&mut self.input[..frames];let output=&mut self.output[..frames];
                for (frame,sample) in input.iter_mut().enumerate() {*sample=block[frame*channels+channel];}
                filter.process(input,output);
                for (frame,sample) in output.iter().enumerate() {block[frame*channels+channel]=*sample;}
            }
        }
    }
    pub fn process(&mut self,samples:&mut [f64])->Result<()> {
        if samples.len()%self.channels.len()!=0 {bail!("incomplete FIR interleaved frame");}
        if !samples.is_empty(){self.apply(samples);self.remaining=self.tail;}
        Ok(())
    }
    /// Only call after the final source. These are filter-tail frames, not programme
    /// frames; playback integrations must not credit them as newly heard source data.
    pub fn drain(&mut self,max_frames:usize)->Vec<f64> {
        let frames=max_frames.min(self.remaining);let mut samples=vec![0.;frames*self.channels.len()];
        self.apply(&mut samples);self.remaining-=frames;samples
    }
    pub fn tail_remaining(&self)->usize{self.remaining}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signal(length:usize)->Vec<f64> {
        let mut state=37_u64;
        (0..length).map(|_|{state=state.wrapping_mul(6364136223846793005).wrapping_add(1);((state>>32) as u32 as f64/u32::MAX as f64-0.5)*0.1}).collect()
    }
    fn reference(input:&[f64],kernel:&[f64])->Vec<f64> {
        let mut out=vec![0.;input.len()+kernel.len()-1];
        for (i,sample) in input.iter().enumerate(){for (j,tap) in kernel.iter().enumerate(){out[i+j]+=sample*tap;}}out
    }
    fn close(actual:&[f64],expected:&[f64]){
        assert_eq!(actual.len(),expected.len());
        for (i,(a,b)) in actual.iter().zip(expected).enumerate(){assert!((a-b).abs()<1e-11,"sample {i}: {a} != {b}");}
    }
    #[test]
    fn all_partition_edges_and_final_tails_match_direct_convolution(){
        for length in [1,2,3,4,5,7,8,15,16,31,32,63,64,65,127,128,129,255,256,257,1025,4097] {
            let input=signal(333);let kernel=signal(length);let expected=reference(&input,&kernel);
            let mut convolver=FirConvolver::new(vec![kernel],1).unwrap();let mut actual=input.clone();convolver.process(&mut actual).unwrap();
            while convolver.tail_remaining()>0 {actual.extend(convolver.drain(113));}
            close(&actual,&expected);assert!(convolver.drain(10).is_empty());
        }
    }
    #[test]
    fn impulse_has_original_gain_phase_and_no_added_block_delay(){
        let mut kernel=signal(8193);kernel[0]=2.;kernel[64]=-3.;kernel[4096]=0.75;
        let mut convolver=FirConvolver::new(vec![kernel.clone()],1).unwrap();let mut input=[1.];convolver.process(&mut input).unwrap();
        assert_eq!(input[0],2.);let mut actual=input.to_vec();actual.extend(convolver.drain(kernel.len()));close(&actual,&kernel);
    }
    #[test]
    fn arbitrary_packet_and_track_boundaries_keep_the_same_history(){
        let input=signal(4097);let kernel=signal(2049);
        let mut full=FirConvolver::new(vec![kernel.clone()],1).unwrap();let mut expected=input.clone();full.process(&mut expected).unwrap();expected.extend(full.drain(9999));
        let mut split=FirConvolver::new(vec![kernel],1).unwrap();let mut actual=Vec::new();let mut cursor=0;
        for size in [1,113,2,997,64,128,17,3,2048,4096] {let end=(cursor+size).min(input.len());let mut chunk=input[cursor..end].to_vec();split.process(&mut chunk).unwrap();actual.extend(chunk);cursor=end;}
        assert_eq!(cursor,input.len());actual.extend(split.drain(9999));assert_eq!(actual,expected);
    }
    #[test]
    fn independent_stereo_kernels_and_mono_expansion_do_not_crossmix(){
        let left=signal(131);let right=vec![0.;131];let first=signal(257);let second=signal(65);
        let mut input=left.iter().zip(&right).flat_map(|(a,b)|[*a,*b]).collect::<Vec<_>>();
        let mut filter=FirConvolver::new(vec![first.clone(),second.clone()],2).unwrap();filter.process(&mut input).unwrap();input.extend(filter.drain(999));
        let expected=reference(&left,&first);close(&input.iter().step_by(2).copied().collect::<Vec<_>>(),&expected);assert!(input.iter().skip(1).step_by(2).all(|v|*v==0.));
        let right=signal(131).into_iter().rev().map(|v|v*0.5).collect::<Vec<_>>();
        let mut input=left.iter().zip(&right).flat_map(|(a,b)|[*a,*b]).collect::<Vec<_>>();
        let mut filter=FirConvolver::new(vec![first,second.clone()],2).unwrap();filter.process(&mut input).unwrap();input.extend(filter.drain(999));
        close(&input.iter().step_by(2).copied().collect::<Vec<_>>(),&expected);
        let mut expected_right=reference(&right,&second);expected_right.resize(expected.len(),0.);
        close(&input.iter().skip(1).step_by(2).copied().collect::<Vec<_>>(),&expected_right);
        let mut mono=FirConvolver::new(vec![vec![2.,-0.5]],2).unwrap();let mut values=[1.,2.,0.,0.];mono.process(&mut values).unwrap();assert_eq!(values,[2.,4.,-0.5,-1.]);
    }
    #[test]
    fn invalid_or_unbounded_filters_and_partial_frames_are_rejected(){
        for kernels in [vec![],vec![vec![]],vec![vec![f64::NAN]],vec![vec![f64::INFINITY]],vec![vec![f64::MAX]],vec![vec![0.;MAX_COEFFICIENTS+1]],vec![vec![1.],vec![1.],vec![1.]]] {assert!(FirConvolver::new(kernels,2).is_err());}
        assert!(FirConvolver::new(vec![vec![0.;MAX_COEFFICIENTS]],32).is_err());
        let mut filter=FirConvolver::new(vec![vec![1.]],2).unwrap();assert!(filter.process(&mut [1.]).is_err());assert_eq!(filter.tail_remaining(),0);
    }
    #[test]
    fn maximum_supported_mono_impulse_keeps_the_complete_response(){
        let kernel=signal(MAX_COEFFICIENTS);let mut filter=FirConvolver::new(vec![kernel.clone()],1).unwrap();
        let mut impulse=[1.];filter.process(&mut impulse).unwrap();let mut actual=impulse.to_vec();
        while filter.tail_remaining()>0 {actual.extend(filter.drain(997));}
        close(&actual,&kernel);
    }
    #[test]
    fn bounded_input_preroll_restores_future_output_without_replaying_programme(){
        let input=signal(4097);let kernel=signal(257);let cut=1999;
        let mut original=FirConvolver::new(vec![kernel.clone()],1).unwrap();let mut before=input[..cut].to_vec();original.process(&mut before).unwrap();
        let mut expected=input[cut..].to_vec();original.process(&mut expected).unwrap();expected.extend(original.drain(9999));
        let mut recovered=FirConvolver::new(vec![kernel.clone()],1).unwrap();let mut history=input[cut-(kernel.len()-1)..cut].to_vec();
        recovered.process(&mut history).unwrap();let mut actual=input[cut..].to_vec();recovered.process(&mut actual).unwrap();actual.extend(recovered.drain(9999));
        close(&actual,&expected);
    }
    #[test]
    fn capped_partitions_wrap_input_history_without_circular_aliasing(){
        let input=signal(MAX_PARTITION*5+3);let mut kernel=vec![0.;MAX_PARTITION*3+1];
        kernel[0]=1.;kernel[MAX_PARTITION]=0.5;kernel[MAX_PARTITION*2]=-0.25;kernel[MAX_PARTITION*3]=0.125;
        let mut expected=vec![0.;input.len()+kernel.len()-1];
        for (i,value) in input.iter().enumerate(){for (offset,gain) in [(0,1.),(MAX_PARTITION,0.5),(MAX_PARTITION*2,-0.25),(MAX_PARTITION*3,0.125)] {expected[i+offset]+=value*gain;}}
        let mut filter=FirConvolver::new(vec![kernel],1).unwrap();let mut actual=input;filter.process(&mut actual).unwrap();
        while filter.tail_remaining()>0 {actual.extend(filter.drain(137));}close(&actual,&expected);
    }
    #[test]
    fn staggered_channels_preserve_complete_responses_across_initial_and_wrapped_blocks(){
        let frames=MAX_PARTITION*5+3;let taps=MAX_PARTITION*3+57;
        let input=signal(frames*4);let mut kernels=Vec::new();let mut expected=vec![0.;(frames+taps-1)*4];
        for channel in 0..4 {
            let mut kernel=vec![0.;taps];
            for (offset,gain) in [(0,1.),(MAX_PARTITION,0.5),(MAX_PARTITION*2,-0.25),(taps-1,0.125)] {
                kernel[offset]=gain*(channel+1) as f64;
                for frame in 0..frames {expected[(frame+offset)*4+channel]+=input[frame*4+channel]*kernel[offset];}
            }
            kernels.push(kernel);
        }
        let mut filter=FirConvolver::new(kernels,4).unwrap();let mut actual=input;
        for chunk in actual.chunks_mut(113*4) {filter.process(chunk).unwrap();}
        while filter.tail_remaining()>0 {actual.extend(filter.drain(137));}close(&actual,&expected);
    }
    #[test]
    fn half_spectrum_keeps_dc_and_nyquist_through_full_tails(){
        let frames=MAX_PARTITION*4+1;let kernel=signal(MAX_PARTITION*2+1);
        let mut dc=vec![0.];let mut nyquist=vec![0.];
        for (index,tap) in kernel.iter().enumerate() {
            dc.push(dc.last().unwrap()+tap);nyquist.push(nyquist.last().unwrap()+tap*if index%2==0{1.}else{-1.});
        }
        let mut actual=(0..frames).flat_map(|frame|[0.25,if frame%2==0{0.125}else{-0.125}]).collect::<Vec<_>>();
        let mut filter=FirConvolver::new(vec![kernel.clone()],2).unwrap();
        for chunk in actual.chunks_mut(131*2) {filter.process(chunk).unwrap();}
        while filter.tail_remaining()>0 {actual.extend(filter.drain(137));}
        let mut expected=Vec::new();
        for frame in 0..frames+kernel.len()-1 {
            let start=(frame+1).saturating_sub(frames);let end=(frame+1).min(kernel.len());
            expected.extend([0.25*(dc[end]-dc[start]),0.125*(if frame%2==0{1.}else{-1.})*(nyquist[end]-nyquist[start])]);
        }
        close(&actual,&expected);
    }
    #[test]
    fn thirty_two_independent_responses_keep_samples_across_internal_blocks_and_full_tail(){
        let channels=32;let frames=MAX_PARTITION*2+5;let taps=MAX_PARTITION*2;
        let input=signal(frames*channels);let mut expected=vec![0.;(frames+taps-1)*channels];let mut kernels=Vec::new();
        for channel in 0..channels {
            let mut kernel=vec![0.;taps];
            for (offset,gain) in [(0,1.+channel as f64/32.),(64,-0.5),(MAX_PARTITION,0.25),(taps-1,0.125)] {
                kernel[offset]=gain;
                for frame in 0..frames {expected[(frame+offset)*channels+channel]+=input[frame*channels+channel]*gain;}
            }
            kernels.push(kernel);
        }
        let mut filter=FirConvolver::new(kernels,channels).unwrap();let mut actual=input;
        filter.process(&mut actual).unwrap();actual.extend(filter.drain(MAX_PARTITION*3+7));close(&actual,&expected);
        assert_eq!(filter.tail_remaining(),0);
    }
}
