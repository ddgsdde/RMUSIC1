import {useMemo,useState} from "react";
import type {Track} from "@r-music/sdk";
import {CatalogSort} from "./CatalogControls";
export const trackSortOptions=["default","title","title_desc","artist","album","added","duration","duration_desc"];
const names=new Intl.Collator(undefined,{numeric:true,sensitivity:"base"});
export function trackOrder<T extends Partial<Track>>(tracks:T[],key:string):number[]{
 const rows=tracks.map((track,index)=>({track,index}));
 if(key!=="default")rows.sort((a,b)=>{
  let difference=0;
  if(key.startsWith("title"))difference=names.compare(a.track.title||"",b.track.title||"");
  else if(key==="artist")difference=names.compare(a.track.artist||"",b.track.artist||"");
  else if(key==="album")difference=names.compare(`${a.track.album_artist||a.track.artist}\0${a.track.album}`,`${b.track.album_artist||b.track.artist}\0${b.track.album}`);
  else if(key==="added")difference=(b.track.created_at||0)-(a.track.created_at||0);
  else if(key.startsWith("duration"))difference=(a.track.duration||0)-(b.track.duration||0);
  return (key.endsWith("_desc")?-difference:difference)||a.index-b.index;
 });
 return rows.map(row=>row.index);
}
export function useTrackSort<T extends Partial<Track>>(tracks:T[],label="显示排序",options=trackSortOptions){
 const [order,setOrder]=useState("default"),indices=useMemo(()=>trackOrder(tracks,order),[tracks,order]);
 return {order,indices,tracks:useMemo(()=>indices.map(index=>tracks[index]),[tracks,indices]),control:<CatalogSort label={label} value={order} change={setOrder} options={options}/>};
}
