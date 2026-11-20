import type {DspConfig,DspEqStage,DspModule} from "@r-music/sdk";

export const maxEqualizers=4, maxChainBands=64;
export type DspSlot={kind:DspModule; index:number};
export type DspFades={fade_in_ms:number;fade_out_ms:number};
export const equalizers=(config:DspConfig):DspEqStage[]=>[
  {bands:config.bands,bypass:!!config.eq_bypass},...config.extra_eq??[],
];
export function withEqualizers(config:DspConfig,stages:DspEqStage[]):DspConfig{
  const first=stages[0];return {...config,bands:first?.bands??[],eq_bypass:first?.bypass||undefined,extra_eq:stages.length>1?stages.slice(1):undefined};
}

export function installedModules(config:DspConfig,fades:DspFades,portable:boolean):Set<DspModule>{
  const installed=new Set<DspModule>(config.modules??[portable?"loudness":"eq"]);
  if(config.bands.length||config.extra_eq?.length)installed.add("eq");
  if(config.preamp_db||config.balance)installed.add("gain");
  if(config.sample_rate)installed.add("rate");
  if(config.stereo)installed.add("stereo");
  if(config.convolution?.response_ids.length)installed.add("convolution");
  if(config.normalize)installed.add("loudness");
  if(fades.fade_in_ms||fades.fade_out_ms)installed.add("fades");
  return installed;
}
