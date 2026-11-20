import type {Endpoint} from "@r-music/sdk";

export function groupRateReason(endpoint:Endpoint|undefined,automatic:boolean):string {
 if(!endpoint?.online)return "输出离线";
 const required=["scheduled_start_v1","continuous_pcm","scheduled_queue_v1","group_output_v2","measured_listens_v2","pcm_rate_adjustment_v1","programme_clock_v2",...(automatic?["group_rate_lease_v1"]:[])];
 return required.some(key=>endpoint.capabilities[key]!==true)?automatic?"此输出暂不支持实验自动校正":"此输出暂不支持实验 PCM 变速":"";
}
