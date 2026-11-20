import {useEffect,useState} from "react";
import type {ProviderManifest,RMusicClient} from "@r-music/sdk";
export function ProviderQuality({api,provider,onError}:{api:RMusicClient;provider:ProviderManifest;onError:(error:unknown)=>void}) {
 const [value,setValue]=useState<string|null>(null),[saving,setSaving]=useState(false);
 useEffect(()=>{let active=true;if(provider.quality_options?.length)api.get<{quality:string}>(`/providers/${encodeURIComponent(provider.id)}/settings`).then(result=>{if(active)setValue(result.quality);}).catch(onError);return()=>{active=false;};},[api,provider.id,onError]);
 if(!provider.quality_options?.length)return null;
 return <label className="provider-quality">在线播放音质<select aria-label={`${provider.name}在线播放音质`} value={value||provider.quality_options[0].id} disabled={value===null||saving} onChange={e=>{const quality=e.target.value;setSaving(true);void api.patch(`/providers/${encodeURIComponent(provider.id)}/settings`,{quality}).then(()=>setValue(quality)).catch(onError).finally(()=>setSaving(false));}}>{provider.quality_options.map(option=><option key={option.id} value={option.id}>{option.label}</option>)}</select><small>仅作用于此音源，下次播放生效。</small></label>;
}
