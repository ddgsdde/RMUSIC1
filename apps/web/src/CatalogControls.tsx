import {useEffect,useState} from 'react';
import type {Track} from '@r-music/sdk';
export const sortNames:Record<string,string>={default:'默认顺序',title:'名称 A–Z',title_desc:'名称 Z–A',artist:'艺术家',album:'专辑曲序',added:'最近收录',release_new:'发行日期：新到旧',release_old:'发行日期：旧到新',duration:'时长：短到长',duration_desc:'时长：长到短',plays:'聆听次数',hot:'平台热度'};
export function CatalogSort({value,change,options,label='排序',defaultLabel}:{value:string;change:(value:string)=>void;options:string[];label?:string;defaultLabel?:string}) {
 return <label className="catalog-sort"><span>{label}</span><select aria-label={label} value={value} onChange={e=>change(e.target.value)}>{options.map(key=><option key={key} value={key}>{(key==="default"?defaultLabel:undefined)||sortNames[key]||key}</option>)}</select></label>;
}
export function releaseDate(t:Track):string {
 const p=t.provider_data as Record<string,any>|undefined;
 return p?.catalog_display?.release_date||p?.release_date||p?.provider_metadata?.background?.release_date||'';
}
export function isNewRelease(date:string,now:number):boolean {
 if(!/^\d{4}-\d{2}-\d{2}$/.test(date))return false;
 const release=Date.parse(date+'T00:00:00Z'),day=Math.floor(now/86400000)*86400000;
 return Number.isFinite(release)&&new Date(release).toISOString().slice(0,10)===date&&release<=day&&day-release<30*86400000;
}
// One hourly clock per visible page, released when navigating away. No per-row timers.
export function useReleaseClock(){const[now,setNow]=useState(Date.now);useEffect(()=>{const id=setInterval(()=>setNow(Date.now()),3600000);return()=>clearInterval(id);},[]);return now;}
export function NewMusic({date,now,album=false}:{date:string;now:number;album?:boolean}){return isNewRelease(date,now)?<small className="new-music" title={`发行于 ${date}，30天内新歌`}>{album?"新专辑":"新歌"}</small>:null;}
export type LibraryFilters=Record<string,string>;
export function filterQuery(value:LibraryFilters){return new URLSearchParams(Object.entries(value).filter(([,v])=>v!=='')).toString();}
export function LibraryFilterBar({value,change,grouped=false}:{value:LibraryFilters;change:(value:LibraryFilters)=>void;grouped?:boolean}){
 const set=(key:string,v:string)=>change({...value,[key]:v});
 const select=(key:string,label:string,options:string[][])=><label>{label}<select aria-label={label} value={value[key]||''} onChange={e=>set(key,e.target.value)}>{options.map(([id,name])=><option key={id} value={id}>{name}</option>)}</select></label>;
 return <div className="library-query-controls">
  <CatalogSort value={value.sort||'default'} change={v=>set('sort',v)} options={grouped?['default','title','title_desc','added','release_new','release_old','plays']:['default','title','title_desc','artist','album','added','release_new','release_old','duration','duration_desc','plays']}/>
  <details><summary>筛选{Object.entries(value).filter(([k,v])=>k!=='sort'&&v).length?` · ${Object.entries(value).filter(([k,v])=>k!=='sort'&&v).length}`:''}</summary><div className="library-filter-grid">
   {grouped&&<label>搜索曲库<input aria-label="搜索曲库" value={value.q||''} onChange={e=>set('q',e.target.value)}/></label>}
   {!grouped&&select('album_status','专辑资料',[['','全部'],['known','已确认专辑'],['unassigned','未归属专辑']])}
   {select('format','音频格式',[['','全部格式'],...['flac','mp3','aac','alac','wav','aiff','ogg','opus','dsf','dff','unknown'].map(v=>[v,v==='unknown'?'未解析':v.toUpperCase()])])}
   {select('listening','聆听记录',[['','全部'],['unplayed','未听过'],['played','已听过']])}
   {select('playlist','播放列表',[['','全部'],['in','在我的播放列表中'],['out','不在我的播放列表中']])}
   {select('artwork','封面资料',[['','全部'],['with','已有封面地址'],['without','未提供封面']])}
   {select('min_rate','最低采样率',[['','不限'],['44100','44.1 kHz'],['48000','48 kHz'],['96000','96 kHz'],['192000','192 kHz']])}
   {select('min_depth','最低位深',[['','不限'],['16','16 bit'],['24','24 bit'],['32','32 bit']])}
   {['released_after','released_before'].map((key,i)=><label key={key}>{i?'发行截止':'发行起始'}<input aria-label={i?'发行截止':'发行起始'} type="date" value={value[key]||''} onChange={e=>set(key,e.target.value)}/></label>)}
   {['min_seconds','max_seconds'].map((key,i)=><label key={key}>{i?'最长时长（秒）':'最短时长（秒）'}<input aria-label={i?'最长时长（秒）':'最短时长（秒）'} type="number" min="0" max="86400" value={value[key]||''} onChange={e=>set(key,e.target.value)}/></label>)}
   <button onClick={()=>change({})}>重置筛选与排序</button>
  </div></details>
 </div>;
}
