import type {DspBand, EqFilter} from "@r-music/sdk";

export const filterNames: Record<EqFilter,string> = {
  peak:"峰值", low_shelf:"低架", high_shelf:"高架", high_pass:"高通",
  low_pass:"低通", band_pass:"带通", notch:"陷波", all_pass:"全通",
};
export const gainFilter = (kind?: EqFilter) => !kind || ["peak","low_shelf","high_shelf"].includes(kind);
export const cutFilter = (kind?: EqFilter) => kind === "low_pass" || kind === "high_pass";
export const displayFrequency = (hz:number) => hz < 1000 ? `${Math.round(hz)} Hz` : `${+(hz/1000).toFixed(2)} kHz`;

export function bandResponse(band:DspBand, rate:number, points:readonly number[]): number[] {
  if (!band.enabled || band.frequency >= rate/2) return points.map(()=>0);
  const w=2*Math.PI*band.frequency/rate, c=Math.cos(w), si=Math.sin(w);
  const a=10**(band.gain_db/40), sum=a+1, diff=a-1;
  const count=cutFilter(band.kind)?(band.slope_db_oct??12)/12:1;
  const sections=Array.from({length:count},(_,i)=>{
    const q=count>1?band.q*Math.SQRT1_2/Math.cos(Math.PI*(2*i+1)/(4*count)):band.q;
    const alpha=si/(2*q), shelf=2*Math.sqrt(a)*alpha;
    switch(band.kind) {
      case "low_shelf":return [a*(sum-diff*c+shelf),2*a*(diff-sum*c),a*(sum-diff*c-shelf),sum+diff*c+shelf,-2*(diff+sum*c),sum+diff*c-shelf];
      case "high_shelf":return [a*(sum+diff*c+shelf),-2*a*(diff+sum*c),a*(sum+diff*c-shelf),sum-diff*c+shelf,2*(diff-sum*c),sum-diff*c-shelf];
      case "low_pass":return [(1-c)/2,1-c,(1-c)/2,1+alpha,-2*c,1-alpha];
      case "high_pass":return [(1+c)/2,-(1+c),(1+c)/2,1+alpha,-2*c,1-alpha];
      case "band_pass":return [alpha,0,-alpha,1+alpha,-2*c,1-alpha];
      case "notch":return [1,-2*c,1,1+alpha,-2*c,1-alpha];
      case "all_pass":return [1-alpha,-2*c,1+alpha,1+alpha,-2*c,1-alpha];
      default:return [1+alpha*a,-2*c,1-alpha*a,1+alpha/a,-2*c,1-alpha/a];
    }
  });
  return points.map(hz=>{
    const w=2*Math.PI*hz/rate, c=Math.cos(w), s=Math.sin(w), c2=Math.cos(2*w), s2=Math.sin(2*w);
    const power=(x:number,y:number,z:number)=>(x+y*c+z*c2)**2+(y*s+z*s2)**2;
    return sections.reduce((db,[b0,b1,b2,a0,a1,a2])=>db+10*Math.log10(Math.max(1e-15,power(b0,b1,b2)/power(a0,a1,a2))),0);
  });
}
