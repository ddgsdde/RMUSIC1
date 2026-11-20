import type {Endpoint,Zone} from "@r-music/sdk";
export interface DeviceGroup {id:string;name:string;zone_id:string;endpoint_id:string;zone_ids:string[];source_zone_id:string;revision:number;phase:string;error?:string|null;job_id?:string|null}
export function syncEligible(endpoint?:Endpoint):boolean {
 return !!endpoint&&endpoint.online&&["coreaudio","wasapi","asio","alsa"].includes(endpoint.kind)&&!["android","ios"].includes(String(endpoint.capabilities.platform||""))&&["playback_instance","scheduled_start_v1","scheduled_queue_v1","group_output_v2","group_output_final_v2","measured_listens_v2","host_clock_v1","device_clock_v1","programme_clock_v1"].every(key=>endpoint.capabilities[key]===true);
}
export function presentedZones(zones:Zone[],endpoints:Endpoint[]):Zone[]{
 const held=new Set(endpoints.filter(e=>e.kind==="group").flatMap(e=>e.group?.zone_ids||[]));
 return zones.filter(z=>!held.has(z.id));
}
