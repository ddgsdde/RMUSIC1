//! Speaker-position stereo fold-down with shared headroom for correlated channels.
use anyhow::{ensure,Result};
#[derive(Clone)]
pub(super) struct Matrix {weights:Vec<[f64;2]>}
impl Matrix {
    pub fn new(channels:u16)->Result<Self> {
        let mask=match channels {1=>4,2=>3,3=>7,4=>0x33,5=>0x37,6=>0x3f,7=>0x70f,8=>0x63f,_=>0};
        Self::layout(channels,mask)
    }
    pub fn layout(channels:u16,mask:u32)->Result<Self> {
        ensure!((1..=8).contains(&channels)&&mask.count_ones()==channels as u32&&mask&!0x3ffff==0,"不支持的多声道布局");
        if channels==1{return Ok(Self{weights:vec![[1.,1.]]});}
        let root=std::f64::consts::FRAC_1_SQRT_2;
        let speakers=[[1.,0.],[0.,1.],[root,root],[0.5,0.5],[root,0.],[0.,root],
            [root,0.],[0.,root],[0.5,0.5],[root,0.],[0.,root],[0.5,0.5],
            [root,0.],[0.5,0.5],[0.,root],[root,0.],[0.5,0.5],[0.,root]];
        let mut weights:Vec<_>=speakers.into_iter().enumerate().filter(|(i,_)|mask&(1<<i)!=0).map(|(_,w)|w).collect();
        let gain=weights.iter().map(|w|w[0]).sum::<f64>().max(weights.iter().map(|w|w[1]).sum::<f64>()).max(1.);
        for pair in &mut weights{pair[0]/=gain;pair[1]/=gain;}
        Ok(Self{weights})
    }
    pub fn mix(&self,frame:&[u8])->[f64;2] {
        let mut result=[0.,0.];
        for (sample,weight)in frame.chunks_exact(4).zip(&self.weights){let sample=i32::from_le_bytes(sample.try_into().unwrap())as f64/2147483648.;result[0]+=sample*weight[0];result[1]+=sample*weight[1];}
        result
    }
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn speaker_masks_preserve_sides_center_and_lfe_without_clipping(){
        for channels in 3..=8 {
            let matrix=Matrix::new(channels).unwrap();
            for channel in 0..channels as usize {
                let mut frame=vec![0;channels as usize*4];frame[channel*4..channel*4+4].copy_from_slice(&i32::MAX.to_le_bytes());
                let mixed=matrix.mix(&frame);assert!(mixed[0]>0.||mixed[1]>0.,"missing channel {channel}/{channels}");
            }
            let all=(0..channels).flat_map(|_|i32::MAX.to_le_bytes()).collect::<Vec<_>>();assert!(matrix.mix(&all).iter().all(|v|*v<=1.));
        }
        let quad=Matrix::new(4).unwrap();let mut rear=vec![0;16];rear[8..12].copy_from_slice(&i32::MAX.to_le_bytes());let sides=quad.mix(&rear);assert!(sides[0]>0.);assert_eq!(sides[1],0.);
        let four_front=Matrix::layout(4,0x0f).unwrap();let mut center=vec![0;16];center[8..12].copy_from_slice(&i32::MAX.to_le_bytes());let mixed=four_front.mix(&center);assert!(mixed[0]>0.);assert_eq!(mixed[0],mixed[1]);
        assert!(Matrix::layout(6,3).is_err());
    }
}
